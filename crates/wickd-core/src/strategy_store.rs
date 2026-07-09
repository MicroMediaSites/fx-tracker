//! The unified `.rhai` strategy store shared by the wickd CLI and the
//! desktop app (AGT-651).
//!
//! ## What the store is
//!
//! A directory of `.rhai` files — by default `~/.wickd/strategies/` — where
//! **the filesystem is the store**. A strategy's canonical identity is its
//! file stem (`revert_adx.rhai` → `revert_adx`), and its metadata
//! (`@parameters` / `@indicators` structured comments) is the single source
//! of truth: nothing is cached in a database, so there is nothing to drift.
//! Both hosts read the same directory:
//!
//! - the CLI resolves bare strategy names to `<store>/<name>.rhai`
//!   (unchanged behavior — the store formalizes the layout the CLI has
//!   always used, so existing files like the live watcher's
//!   `revert_adx.rhai` keep resolving at their current paths), and
//! - the desktop app lists/reads the same directory read-only (it is a
//!   viewer/runner; authoring happens through the CLI).
//!
//! ## Layout rules
//!
//! - Only top-level `*.rhai` files are strategies. Subdirectories (e.g. an
//!   `attic/` for parked strategies) are ignored by `list()`.
//! - Names are slugs: ASCII alphanumerics plus `-`, `_` and `.` — no path
//!   separators, so a name can never escape the store directory.
//! - Writes are additive and explicit: `add` refuses to overwrite unless
//!   asked, and nothing in the store ever renames or rewrites files behind
//!   the user's back (a live watcher may be holding one of these paths).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::Serialize;
use shared::{IndicatorConfig, ParameterDefinition};

use crate::backtest::validate_script;
use crate::paths::wickd_data_home;

/// One strategy in the store. `valid`/`error` come from running the script
/// through `validate_script` at list/read time, so hosts can render broken
/// scripts without refusing to list them.
#[derive(Debug, Clone, Serialize)]
pub struct StoredStrategy {
    /// Canonical name (file stem) — the id both hosts use.
    pub name: String,
    /// Absolute path of the `.rhai` file.
    pub path: PathBuf,
    /// Whether the script passes `validate_script`.
    pub valid: bool,
    /// Validation error when `valid` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Parameters declared in the script's `@parameters` metadata.
    pub parameters: Vec<ParameterDefinition>,
    /// Indicators declared in the script's `@indicators` metadata.
    pub indicators: Vec<IndicatorConfig>,
    /// File modification time (epoch milliseconds), 0 when unavailable.
    pub modified_at: i64,
    /// Stable content fingerprint (used for dedupe during imports).
    pub content_hash: String,
}

/// A directory-backed store of `.rhai` strategies.
#[derive(Debug, Clone)]
pub struct StrategyStore {
    root: PathBuf,
}

impl StrategyStore {
    /// Open the default store at `<data home>/strategies`
    /// (`~/.wickd/strategies`, or `$WICKD_HOME/strategies` in tests).
    pub fn open_default() -> Result<Self, String> {
        Ok(Self {
            root: wickd_data_home()?.join("strategies"),
        })
    }

    /// Open a store rooted at an explicit directory (tests, tooling).
    pub fn open_at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The store's root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The canonical path for a strategy name — `<root>/<name>.rhai`.
    /// This is byte-for-byte the CLI's historical bare-name resolution, so
    /// pre-store files keep resolving unchanged.
    pub fn path_for(&self, name: &str) -> PathBuf {
        self.root.join(format!("{name}.rhai"))
    }

    /// Validate a strategy name: non-empty slug of ASCII alphanumerics,
    /// `-`, `_`, `.` — and no leading dot (hidden files / `..` traversal).
    pub fn validate_name(name: &str) -> Result<(), String> {
        if name.is_empty() {
            return Err("strategy name must not be empty".to_string());
        }
        if name.starts_with('.') {
            return Err(format!("invalid strategy name '{name}': must not start with '.'"));
        }
        if let Some(bad) = name
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
        {
            return Err(format!(
                "invalid strategy name '{name}': character '{bad}' is not allowed \
                 (use ASCII letters, digits, '-', '_', '.')"
            ));
        }
        Ok(())
    }

    /// Turn a free-form display name into a store slug (for imports and
    /// conversion tooling): lowercase, runs of disallowed characters become
    /// single '_', trimmed.
    pub fn slugify(display_name: &str) -> String {
        let mut out = String::with_capacity(display_name.len());
        let mut last_was_sep = true; // trim leading separators
        for c in display_name.chars() {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '.') {
                out.push(c);
                last_was_sep = false;
            } else if !last_was_sep {
                out.push('_');
                last_was_sep = true;
            }
        }
        while out.ends_with('_') {
            out.pop();
        }
        if out.starts_with('.') {
            out.insert(0, '_');
        }
        if out.is_empty() {
            "strategy".to_string()
        } else {
            out
        }
    }

    /// List all strategies (top-level `*.rhai` files), sorted by name.
    /// A missing store directory is an empty store, not an error.
    pub fn list(&self) -> Result<Vec<StoredStrategy>, String> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(format!("reading store {}: {e}", self.root.display())),
        };
        for entry in entries {
            let entry = entry.map_err(|e| format!("reading store {}: {e}", self.root.display()))?;
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("rhai") {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|s| s.to_str()).map(String::from) else {
                continue;
            };
            out.push(self.entry_from_path(name, path)?);
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Read one strategy plus its source. `Ok(None)` when the name has no
    /// file in the store.
    pub fn read(&self, name: &str) -> Result<Option<(StoredStrategy, String)>, String> {
        Self::validate_name(name)?;
        let path = self.path_for(name);
        if !path.is_file() {
            return Ok(None);
        }
        let source = std::fs::read_to_string(&path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        let entry = self.entry_from_source(name.to_string(), path, &source);
        Ok(Some((entry, source)))
    }

    /// Add a strategy to the store. The script must pass `validate_script`.
    /// Refuses to replace an existing file unless `overwrite` is set — and
    /// even then writes only the named file, never touching siblings.
    pub fn add(&self, name: &str, source: &str, overwrite: bool) -> Result<StoredStrategy, String> {
        Self::validate_name(name)?;
        validate_script(source).map_err(|e| format!("invalid strategy script: {e}"))?;
        let path = self.path_for(name);
        if path.exists() && !overwrite {
            return Err(format!(
                "strategy '{name}' already exists at {} (use overwrite/update to replace it)",
                path.display()
            ));
        }
        std::fs::create_dir_all(&self.root)
            .map_err(|e| format!("creating store {}: {e}", self.root.display()))?;
        std::fs::write(&path, source).map_err(|e| format!("writing {}: {e}", path.display()))?;
        Ok(self.entry_from_source(name.to_string(), path, source))
    }

    /// Remove a strategy file. Returns `false` when it did not exist.
    pub fn remove(&self, name: &str) -> Result<bool, String> {
        Self::validate_name(name)?;
        let path = self.path_for(name);
        if !path.is_file() {
            return Ok(false);
        }
        std::fs::remove_file(&path).map_err(|e| format!("removing {}: {e}", path.display()))?;
        Ok(true)
    }

    fn entry_from_path(&self, name: String, path: PathBuf) -> Result<StoredStrategy, String> {
        let source = std::fs::read_to_string(&path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        Ok(self.entry_from_source(name, path, &source))
    }

    fn entry_from_source(&self, name: String, path: PathBuf, source: &str) -> StoredStrategy {
        let modified_at = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        match validate_script(source) {
            Ok(meta) => StoredStrategy {
                name,
                path,
                valid: true,
                error: None,
                parameters: meta.parameters,
                indicators: meta.indicators,
                modified_at,
                content_hash: content_hash(source),
            },
            Err(e) => StoredStrategy {
                name,
                path,
                valid: false,
                error: Some(e),
                parameters: Vec::new(),
                indicators: Vec::new(),
                modified_at,
                content_hash: content_hash(source),
            },
        }
    }
}

/// Stable content fingerprint for dedupe (not cryptographic).
pub fn content_hash(source: &str) -> String {
    let mut h = DefaultHasher::new();
    source.hash(&mut h);
    format!("{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_SCRIPT: &str = r#"
// @parameters: [ { "id": "period", "name": "Period", "type": "integer", "default": 14 } ]
fn on_candle() {
    let p = param("period");
    "hold"
}
"#;

    fn temp_store() -> (StrategyStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "wickd-store-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        (StrategyStore::open_at(&dir), dir)
    }

    #[test]
    fn missing_store_dir_lists_empty() {
        let (store, dir) = temp_store();
        assert!(store.list().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn add_read_list_remove_roundtrip() {
        let (store, dir) = temp_store();
        let entry = store.add("my-strat", VALID_SCRIPT, false).unwrap();
        assert!(entry.valid);
        assert_eq!(entry.parameters.len(), 1);
        assert_eq!(entry.path, dir.join("my-strat.rhai"));

        // add without overwrite refuses to clobber
        let err = store.add("my-strat", VALID_SCRIPT, false).unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        // overwrite succeeds
        store.add("my-strat", VALID_SCRIPT, true).unwrap();

        let (read, source) = store.read("my-strat").unwrap().unwrap();
        assert_eq!(source, VALID_SCRIPT);
        assert_eq!(read.name, "my-strat");

        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "my-strat");

        assert!(store.remove("my-strat").unwrap());
        assert!(!store.remove("my-strat").unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn add_rejects_invalid_scripts_and_bad_names() {
        let (store, dir) = temp_store();
        // invalid script never lands on disk
        let err = store.add("broken", "fn nope() {}", false).unwrap_err();
        assert!(err.contains("invalid strategy script"), "{err}");
        assert!(!dir.join("broken.rhai").exists());
        // path traversal / separators rejected
        assert!(StrategyStore::validate_name("../evil").is_err());
        assert!(StrategyStore::validate_name("a/b").is_err());
        assert!(StrategyStore::validate_name(".hidden").is_err());
        assert!(StrategyStore::validate_name("ok-name_1.v2").is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn list_marks_invalid_scripts_but_still_lists_them() {
        let (store, dir) = temp_store();
        store.add("good", VALID_SCRIPT, false).unwrap();
        // Simulate a hand-dropped broken file (the store itself refuses to
        // write invalid scripts, but users can drop anything in the dir).
        std::fs::write(dir.join("broken.rhai"), "fn nope() {}").unwrap();
        // Subdirectories are ignored.
        std::fs::create_dir_all(dir.join("attic")).unwrap();
        std::fs::write(dir.join("attic").join("parked.rhai"), VALID_SCRIPT).unwrap();

        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 2);
        let broken = listed.iter().find(|s| s.name == "broken").unwrap();
        assert!(!broken.valid);
        assert!(broken.error.is_some());
        let good = listed.iter().find(|s| s.name == "good").unwrap();
        assert!(good.valid);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn slugify_produces_valid_names() {
        for (input, want) in [
            ("Ichi w MACD Confirm (v2)", "ichi_w_macd_confirm_v2"),
            ("  Mean Reversion  ", "mean_reversion"),
            ("EUR/USD H1", "eur_usd_h1"),
            ("", "strategy"),
        ] {
            let got = StrategyStore::slugify(input);
            assert_eq!(got, want);
            StrategyStore::validate_name(&got).unwrap();
        }
    }

    #[test]
    fn content_hash_is_stable_and_discriminating() {
        assert_eq!(content_hash("abc"), content_hash("abc"));
        assert_ne!(content_hash("abc"), content_hash("abd"));
    }
}
