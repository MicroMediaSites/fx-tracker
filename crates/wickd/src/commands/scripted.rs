//! Shared resolution for Rhai-scripted strategies.
//!
//! `wickd strategy run` and `wickd backtest` both take a single `strategy`
//! argument that is either a built-in name (`ma-crossover`, `rsi`) or a
//! `.rhai` script — as an explicit file path, or a bare name looked up in
//! `~/.wickd/strategies/<name>.rhai`. This module is the one place that
//! resolves that argument to a script file and turns it into a `Strategy`,
//! so both call sites stay consistent instead of re-deriving the rules.
//!
//! The Rhai engine itself (`ScriptedStrategy`, `validate_script`) lives in
//! `wickd-core::backtest::scripted_strategy` — this module only wires
//! it to the CLI's path resolution + error conventions.
//!
//! Resolution is split into two steps so callers can keep built-in names
//! (`ma-crossover`, `rsi`) taking precedence — per the ticket, "a known
//! built-in name keeps existing behavior unchanged":
//! 1. [`resolve_explicit_script_path`] — an unambiguous script reference
//!    (an existing literal file, or anything ending in `.rhai`). Callers
//!    should check this *before* matching built-in names, since the user's
//!    intent is unambiguous either way.
//! 2. [`resolve_named_script_path`] — a bare name looked up under
//!    `~/.wickd/strategies/`. Callers should only check this *after* built-in
//!    names have been ruled out, so a script can't shadow a built-in.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use wickd_core::backtest::{validate_script, ScriptedStrategy, Strategy};
use shared::{ParameterDefinition, ParameterType};

/// Resolve `name_or_path` to a `.rhai` script file when the reference is
/// unambiguous: either it already exists as a literal file on disk, or it
/// ends in `.rhai` (treated as a script path even if missing, so a typo'd
/// path produces a clear "file not found" error rather than falling through
/// to "unknown strategy"). Returns `None` otherwise — callers should then
/// try built-in name matching, and finally [`resolve_named_script_path`].
pub fn resolve_explicit_script_path(name_or_path: &str) -> Option<PathBuf> {
    let literal = Path::new(name_or_path);
    if literal.is_file() {
        return Some(literal.to_path_buf());
    }
    if name_or_path.ends_with(".rhai") {
        return Some(literal.to_path_buf());
    }
    None
}

/// Resolve a bare `name` to `~/.wickd/strategies/<name>.rhai` if that file
/// exists. Callers should only reach this once `name` has already failed to
/// match a built-in strategy, so scripts never shadow built-ins.
///
/// Resolution goes through the unified [`StrategyStore`] (AGT-651) — same
/// path as always (`<store>/<name>.rhai`), so pre-store files (including the
/// live watcher's strategies) keep resolving unchanged.
pub fn resolve_named_script_path(name: &str) -> Result<Option<PathBuf>> {
    let store = wickd_core::strategy_store::StrategyStore::open_default()
        .map_err(anyhow::Error::msg)?;
    let named = store.path_for(name);
    if named.is_file() {
        return Ok(Some(named));
    }
    Ok(None)
}

/// Parse repeatable `--set <id>=<value>` pairs into a parameter-override map.
/// Pure and strict: malformed pairs, non-numeric values, and duplicate ids
/// are all errors (the "parameter" token routes them to `exit::VALIDATION`).
pub fn parse_set_pairs(pairs: &[String]) -> Result<HashMap<String, f64>> {
    let mut out = HashMap::new();
    for pair in pairs {
        let (id, raw) = pair.split_once('=').ok_or_else(|| {
            anyhow::anyhow!("invalid parameter override '--set {pair}': expected <id>=<value>")
        })?;
        let id = id.trim();
        if id.is_empty() {
            bail!("invalid parameter override '--set {pair}': empty parameter id");
        }
        let value: f64 = raw.trim().parse().map_err(|_| {
            anyhow::anyhow!(
                "invalid parameter override '--set {pair}': '{}' is not a number",
                raw.trim()
            )
        })?;
        if out.insert(id.to_string(), value).is_some() {
            bail!("duplicate '--set' for parameter '{id}'");
        }
    }
    Ok(out)
}

/// Validate overrides against the script's declared `@parameters`: unknown
/// ids, out-of-min/max values, and fractional values for integer parameters
/// are structured errors rather than silent no-ops.
fn validate_overrides(
    overrides: &HashMap<String, f64>,
    defs: &[ParameterDefinition],
    path: &Path,
) -> Result<()> {
    for (id, value) in overrides {
        let def = defs.iter().find(|d| d.id == *id).ok_or_else(|| {
            let declared = if defs.is_empty() {
                "(none)".to_string()
            } else {
                defs.iter().map(|d| d.id.as_str()).collect::<Vec<_>>().join(", ")
            };
            anyhow::anyhow!(
                "unknown parameter '{id}' for '{}' — @parameters declares: {declared}",
                path.display()
            )
        })?;
        if let Some(min) = def.min {
            if *value < min {
                bail!("parameter '{id}' value {value} is below the declared min {min}");
            }
        }
        if let Some(max) = def.max {
            if *value > max {
                bail!("parameter '{id}' value {value} is above the declared max {max}");
            }
        }
        if def.param_type == ParameterType::Integer && value.fract() != 0.0 {
            bail!("parameter '{id}' is an integer parameter; got {value}");
        }
    }
    Ok(())
}

/// Load, validate, and construct a scripted strategy from `path` for
/// `instrument`, applying `--set` parameter overrides. Returns the strategy
/// plus its EFFECTIVE parameter map (defaults merged with overrides) so runs
/// are self-describing. Never panics on a malformed script — every Rhai
/// engine error (compile, missing `on_candle`, metadata parse) is converted
/// into a plain `anyhow::Error` with a message that names the offending
/// file, so the CLI's existing `strategy_failed` / `backtest_failed` error
/// envelope and `exit::VALIDATION` code apply unchanged.
pub fn load_scripted_strategy(
    path: &Path,
    instrument: &str,
    overrides: &HashMap<String, f64>,
) -> Result<(Box<dyn Strategy>, serde_json::Value)> {
    let script = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read strategy script '{}'", path.display()))?;

    // Validate first (compiles the script, checks for `on_candle`, parses
    // `@indicators`/`@parameters` metadata) so a malformed script surfaces a
    // clear message here instead of panicking or failing silently later.
    validate_script(&script)
        .map_err(|e| anyhow::anyhow!("invalid strategy script '{}': {e}", path.display()))?;

    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("scripted")
        .to_string();

    // One-stop host construction (AGT-651): parameters, pip value, event
    // calendar (ABI v3, ~/.wickd/events.json else bundled) and surprise feed
    // (ABI v4, ~/.wickd/calendar/*.csv) are all wired inside for_host — the
    // desktop app constructs through the same path, so host wiring can't
    // drift between the CLI and the app.
    let strategy = ScriptedStrategy::for_host(&script, &name, overrides.clone(), instrument)
        .map_err(|e| anyhow::anyhow!("failed to load strategy script '{}': {e}", path.display()))?;
    // for_host silently ignores unknown override keys, so validate
    // explicitly — a typo'd --set must be an error, not a no-op.
    validate_overrides(overrides, strategy.get_parameters(), path)?;

    let effective =
        serde_json::to_value(strategy.get_resolved_params()).unwrap_or(serde_json::Value::Null);
    Ok((Box::new(strategy), effective))
}

/// Load and validate a `.rhai` script for the live watcher (`wickd watch`,
/// AGT-624), applying `--set` parameter overrides with exactly the same
/// validation as backtest. Unlike [`load_scripted_strategy`] this is
/// instrument-agnostic — the watcher constructs one `ScriptedStrategy` per
/// watchlist instrument itself (with per-instrument pip value + event
/// calendar), so this only proves the script + overrides are valid and
/// returns the raw script source plus the EFFECTIVE parameter map (defaults
/// merged with overrides) so the run is self-describing. Every failure names
/// the offending file (AC4).
pub fn load_validated_script(
    path: &Path,
    overrides: &HashMap<String, f64>,
) -> Result<(String, serde_json::Value)> {
    let script = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read strategy script '{}'", path.display()))?;

    validate_script(&script)
        .map_err(|e| anyhow::anyhow!("invalid strategy script '{}': {e}", path.display()))?;

    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("scripted")
        .to_string();

    let strategy = ScriptedStrategy::from_script_with_params(&script, &name, overrides.clone())
        .map_err(|e| anyhow::anyhow!("failed to load strategy script '{}': {e}", path.display()))?;
    // from_script_with_params silently ignores unknown override keys, so
    // validate explicitly — a typo'd --set must be an error, not a no-op.
    validate_overrides(overrides, strategy.get_parameters(), path)?;

    let effective =
        serde_json::to_value(strategy.get_resolved_params()).unwrap_or(serde_json::Value::Null);
    Ok((script, effective))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A `.rhai` file under the OS temp dir, deleted when it drops. Mirrors
    /// the manual temp-path pattern used in `pending.rs`/`audit.rs` tests
    /// (this crate doesn't otherwise depend on the `tempfile` crate).
    struct TempScript(PathBuf);

    impl TempScript {
        fn new(contents: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let mut p = std::env::temp_dir();
            p.push(format!("wickd-scripted-test-{pid}-{nanos}-{n}.rhai"));
            std::fs::write(&p, contents).expect("write temp script");
            Self(p)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempScript {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn write_temp_script(contents: &str) -> TempScript {
        TempScript::new(contents)
    }

    const VALID_SCRIPT: &str = r#"
fn on_candle() {
    "hold"
}
"#;

    const MALFORMED_SCRIPT: &str = r#"
fn on_candle( {
    "hold"
}
"#;

    const MISSING_ON_CANDLE_SCRIPT: &str = r#"
fn not_on_candle() {
    "hold"
}
"#;

    #[test]
    fn resolve_explicit_script_path_finds_literal_existing_file() {
        let f = write_temp_script(VALID_SCRIPT);
        let path = f.path().to_str().unwrap();
        let resolved = resolve_explicit_script_path(path);
        assert_eq!(resolved.unwrap(), f.path().to_path_buf());
    }

    #[test]
    fn resolve_explicit_script_path_treats_dot_rhai_suffix_as_a_script_even_if_missing() {
        let resolved = resolve_explicit_script_path("/nonexistent/dir/some-strategy.rhai");
        assert_eq!(
            resolved.unwrap(),
            PathBuf::from("/nonexistent/dir/some-strategy.rhai")
        );
    }

    #[test]
    fn resolve_explicit_script_path_returns_none_for_builtin_names() {
        assert!(resolve_explicit_script_path("ma-crossover").is_none());
        assert!(resolve_explicit_script_path("rsi").is_none());
    }

    #[test]
    fn resolve_named_script_path_returns_none_when_no_matching_file_under_wickd_strategies() {
        // Won't match any real ~/.wickd/strategies/ file in a test environment.
        assert!(resolve_named_script_path("definitely-not-a-real-strategy-name-xyz")
            .unwrap()
            .is_none());
    }

    #[test]
    fn load_scripted_strategy_succeeds_for_a_valid_script() {
        let f = write_temp_script(VALID_SCRIPT);
        let strategy = load_scripted_strategy(f.path(), "EUR_USD", &HashMap::new());
        assert!(strategy.is_ok());
    }

    #[test]
    fn load_scripted_strategy_reports_a_clear_error_for_a_compile_failure() {
        let f = write_temp_script(MALFORMED_SCRIPT);
        let err = load_scripted_strategy(f.path(), "EUR_USD", &HashMap::new()).err().unwrap();
        let msg = format!("{err:#}");
        assert!(msg.contains("invalid strategy script"), "message was: {msg}");
    }

    #[test]
    fn load_scripted_strategy_reports_a_clear_error_when_on_candle_is_missing() {
        let f = write_temp_script(MISSING_ON_CANDLE_SCRIPT);
        let err = load_scripted_strategy(f.path(), "EUR_USD", &HashMap::new()).err().unwrap();
        let msg = format!("{err:#}");
        assert!(msg.contains("invalid strategy script"), "message was: {msg}");
        assert!(msg.contains("on_candle"), "message was: {msg}");
    }

    #[test]
    fn load_scripted_strategy_reports_a_clear_error_for_a_missing_file() {
        let err = load_scripted_strategy(
            Path::new("/nonexistent/does-not-exist.rhai"),
            "EUR_USD",
            &HashMap::new(),
        )
        .err()
        .unwrap();
        let msg = format!("{err:#}");
        assert!(msg.contains("failed to read strategy script"), "message was: {msg}");
    }

    // ---- --set parameter overrides (#291) ----

    const PARAMETERIZED_SCRIPT: &str = r#"
// @parameters: [
//   { "id": "threshold", "type": "number", "default": 30.0, "min": 10.0, "max": 90.0 },
//   { "id": "lookback", "type": "integer", "default": 14, "min": 2, "max": 100 }
// ]
fn on_candle() {
    if param("threshold") > 0.0 { "hold" } else { "hold" }
}
"#;

    #[test]
    fn parse_set_pairs_accepts_valid_and_rejects_malformed() {
        let ok = parse_set_pairs(&["a=1".into(), "b=2.5".into(), "c=-3".into()]).unwrap();
        assert_eq!(ok.get("a"), Some(&1.0));
        assert_eq!(ok.get("b"), Some(&2.5));
        assert_eq!(ok.get("c"), Some(&-3.0));
        // Whitespace around id/value is tolerated.
        assert_eq!(parse_set_pairs(&[" x = 7 ".into()]).unwrap().get("x"), Some(&7.0));

        assert!(parse_set_pairs(&["no-equals".into()]).is_err());
        assert!(parse_set_pairs(&["=5".into()]).is_err()); // empty id
        assert!(parse_set_pairs(&["x=abc".into()]).is_err()); // non-numeric
        assert!(parse_set_pairs(&["x=1".into(), "x=2".into()]).is_err()); // duplicate
        assert!(parse_set_pairs(&[]).unwrap().is_empty());
    }

    #[test]
    fn overrides_apply_and_echo_in_effective_params() {
        let f = write_temp_script(PARAMETERIZED_SCRIPT);
        let overrides = parse_set_pairs(&["threshold=55".into()]).unwrap();
        let (_s, effective) = load_scripted_strategy(f.path(), "EUR_USD", &overrides).unwrap();
        assert_eq!(effective["threshold"], 55.0);
        // Untouched params echo their defaults — the run is self-describing.
        assert_eq!(effective["lookback"], 14.0);
    }

    #[test]
    fn overrides_are_validated_against_declared_parameters() {
        let f = write_temp_script(PARAMETERIZED_SCRIPT);

        // Unknown id: an error naming the declared parameters, not a no-op.
        let overrides = parse_set_pairs(&["typo=5".into()]).unwrap();
        let msg = format!("{:#}", load_scripted_strategy(f.path(), "EUR_USD", &overrides).err().unwrap());
        assert!(msg.contains("unknown parameter 'typo'"), "message was: {msg}");
        assert!(msg.contains("threshold, lookback"), "message was: {msg}");

        // Out of declared range.
        let overrides = parse_set_pairs(&["threshold=95".into()]).unwrap();
        let msg = format!("{:#}", load_scripted_strategy(f.path(), "EUR_USD", &overrides).err().unwrap());
        assert!(msg.contains("above the declared max"), "message was: {msg}");
        let overrides = parse_set_pairs(&["threshold=5".into()]).unwrap();
        let msg = format!("{:#}", load_scripted_strategy(f.path(), "EUR_USD", &overrides).err().unwrap());
        assert!(msg.contains("below the declared min"), "message was: {msg}");

        // Fractional value for an integer parameter.
        let overrides = parse_set_pairs(&["lookback=14.5".into()]).unwrap();
        let msg = format!("{:#}", load_scripted_strategy(f.path(), "EUR_USD", &overrides).err().unwrap());
        assert!(msg.contains("integer parameter"), "message was: {msg}");
    }
}
