//! The app as a reader of the CLI's multi-account performance glance.
//!
//! The desktop app's own credential vault is single-account (one API key, one
//! account id, in `~/.wickd/app.db`). The *CLI* is the multi-account side:
//! `~/.wickd/config.json` holds every named account (`h004`, `tf-m1`, …) and
//! the keys live in the OS keychain. Rather than duplicate that resolution here
//! — `vault_store` lives in the `wickd` binary crate, not importable from the
//! app — this shells out to `wickd trade glance`, the same trust boundary
//! `feed_ask` uses: the CLI owns credentials and OANDA, the app only renders.
//!
//! Unlike the feed/calendar readers this one is NOT offline: it hits OANDA
//! through the CLI and takes ~5s for a full account fan-out. So it is cached
//! with a TTL and the UI is expected to render the last known value while a
//! refresh runs — never to block a panel on it. It is never on the boot path.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::daemon::find_wickd_binary;

/// How long a fetched glance stays fresh. The underlying numbers move only when
/// a trade closes, so a minute of staleness is invisible in practice and keeps
/// a re-rendering panel from re-hitting OANDA.
const CACHE_TTL: Duration = Duration::from_secs(60);

/// Hard ceiling on the CLI call. The fan-out is one round trip per account in
/// parallel (~5s observed for 6 accounts); 90s only trips if OANDA is hanging.
const GLANCE_TIMEOUT: Duration = Duration::from_secs(90);

/// One account's row. Every metric is optional because a row whose fetch failed
/// carries only `account`/`names`/`error` — one revoked key must not blank the
/// whole panel, so failures are per-row, not per-request.
///
/// Money crosses as strings (exact decimals, never lossy floats) — same
/// convention as the CLI's audit ledger and backtest metrics.
/// One open position, passed through verbatim from the CLI (exact decimal
/// strings, same convention as everything else here).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenPosition {
    pub instrument: String,
    /// Net signed units (negative = short).
    pub units: String,
    pub unrealized_pl: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountGlance {
    /// Primary display name (the informative one when an account is aliased).
    pub account: String,
    /// Every configured name resolving to this OANDA account, primary first.
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    /// OANDA's own account name — what the broker dashboard shows. This layer
    /// re-serializes the CLI's JSON, so any field missing HERE is silently
    /// stripped before the frontend sees it (how the alias went missing on
    /// 2026-08-06).
    #[serde(default)]
    pub alias: Option<String>,
    /// Open positions; None = the CLI's fetch failed (UI falls back to the
    /// bare count), Some([]) = genuinely flat. Keep the distinction.
    #[serde(default)]
    pub open_positions: Option<Vec<OpenPosition>>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub nav: Option<String>,
    #[serde(default)]
    pub balance: Option<String>,
    #[serde(default)]
    pub unrealized_pl: Option<String>,
    #[serde(default)]
    pub open_trade_count: Option<i32>,
    /// Realized P&L summed over the window.
    #[serde(default)]
    pub realized: Option<String>,
    #[serde(default)]
    pub trades: Option<u64>,
    #[serde(default)]
    pub wins: Option<u64>,
    #[serde(default)]
    pub losses: Option<u64>,
    /// Null when nothing was decided in the window — render "—", not 0%.
    #[serde(default)]
    pub win_rate: Option<f64>,
    /// This row's own window start (RFC3339), or null when the row is
    /// unmeasured (`--since-baseline` for an un-baselined account, D3).
    #[serde(default)]
    pub window_start: Option<String>,
    /// Which input decided this row's window: "baseline" | "since" | "days".
    #[serde(default)]
    pub window_source: Option<String>,
    /// Why the row is unmeasured, when it is (e.g. "no baseline recorded").
    /// Null on the ordinary path.
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountsGlance {
    pub environment: String,
    /// Null when an explicit `since`/`since_baseline` drove the window (the
    /// app's "Today", or "since baseline"), since no whole-day count
    /// describes either.
    #[serde(default)]
    pub days: Option<u32>,
    /// Start of the window (RFC3339). Null under `--since-baseline`: the
    /// window is per-account there (AGT-1128) — each row's own `window_start`
    /// is authoritative and there is no single shared start.
    #[serde(default)]
    pub since: Option<String>,
    /// The shared exclusive upper bound (AGT-1129, D4). Always present.
    #[serde(default)]
    pub to: Option<String>,
    /// When the CLI produced these numbers (RFC3339) — the UI shows this as the
    /// as-of stamp so a cached render never looks live when it isn't.
    pub generated_at: String,
    #[serde(default)]
    pub accounts: Vec<AccountGlance>,
}

/// Cache identity: environment + the exact window requested (D9). `since`,
/// `to`, and `since_baseline` are each part of the key alongside `days` — the
/// app's "Today" reuses one `since` value all day (keying on `days` alone
/// would serve a stale midnight boundary after the date rolls over), and a
/// `--since-baseline` request must not collide with a `days`/`since` one that
/// happens to share the same env, nor with a different `--to`.
type CacheKey = (String, Option<u32>, Option<String>, Option<String>, bool);

struct Cached {
    key: CacheKey,
    value: AccountsGlance,
    fetched: Instant,
}

/// Serialized so two panels mounting at once produce one CLI call, not two.
static CACHE: OnceLock<Mutex<Option<Cached>>> = OnceLock::new();

fn cache() -> &'static Mutex<Option<Cached>> {
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Validate an RFC3339 instant for use as a CLI argument. Failing here (rather
/// than letting the CLI reject it) keeps the error specific to the field and
/// input that was actually wrong, and guarantees nothing user-typed reaches
/// argv unparsed (D10).
fn validate_rfc3339(field: &str, value: &str) -> Result<String, String> {
    chrono::DateTime::parse_from_rfc3339(value.trim())
        .map(|d| d.to_rfc3339())
        .map_err(|e| format!("invalid {field} '{value}': {e}"))
}

/// Build the `wickd trade glance` argv for one resolved window (D9/D10, AC1).
///
/// `since`/`days` are expected already normalized by the caller: under
/// `since_baseline` both are `None` (the flag alone drives the window — the
/// CLI's own `conflicts_with` would reject `--since-baseline` alongside
/// `--since`/`--days`, so this never emits them together, mirroring that
/// exclusivity without needing to duplicate it). `to`, when given, composes
/// with every mode (AGT-1129 D4) — including `since_baseline`.
fn build_glance_args(
    env: &str,
    days: Option<u32>,
    since: Option<&str>,
    to: Option<&str>,
    since_baseline: bool,
) -> Vec<String> {
    let mut args: Vec<String> = vec!["trade".into(), "glance".into(), "--env".into(), env.into()];
    if since_baseline {
        args.push("--since-baseline".into());
    } else {
        match (since, days) {
            (Some(s), _) => args.extend(["--since".to_string(), s.to_string()]),
            (None, Some(d)) => args.extend(["--days".to_string(), d.to_string()]),
            (None, None) => {
                unreachable!("days is always Some when since is None and since_baseline is false")
            }
        }
    }
    if let Some(t) = to {
        args.extend(["--to".to_string(), t.to_string()]);
    }
    args
}

/// Rolling-window performance for every account configured in `env`.
///
/// `refresh: true` bypasses the TTL (the panel's manual refresh button).
/// `since`, when given, is an RFC3339 instant that overrides `days` — the
/// frontend passes its own local midnight for the "Today" window, which no
/// whole-day count can express. `since_baseline: true` starts each account at
/// its OWN recorded baseline instead (D2) — it ignores `days`/`since`
/// entirely (AC1), matching the CLI's `--since-baseline`, which is
/// `conflicts_with` both. `to`, when given, closes the window (D4) and
/// composes with every mode, `since_baseline` included.
#[tauri::command]
pub async fn accounts_glance(
    days: Option<u32>,
    since: Option<String>,
    to: Option<String>,
    since_baseline: Option<bool>,
    env: Option<String>,
    refresh: Option<bool>,
) -> Result<AccountsGlance, String> {
    let env = match env.as_deref().unwrap_or("practice") {
        // Allowlist, not passthrough: this string becomes a CLI argument.
        e @ ("practice" | "live") => e.to_string(),
        other => return Err(format!("unknown environment '{other}'")),
    };
    let since_baseline = since_baseline.unwrap_or(false);
    // Validate before anything becomes an argument: the CLI would reject a
    // malformed instant anyway, but failing here keeps the error specific to
    // the input (D10).
    let since = match since {
        Some(s) => Some(validate_rfc3339("since", &s)?),
        None => None,
    };
    let to = match to {
        Some(t) => Some(validate_rfc3339("to", &t)?),
        None => None,
    };
    // `since_baseline` ignores `days`/`since` (AC1) — normalize both to `None`
    // here so a `--since-baseline` request never carries a stale/irrelevant
    // `days` or `since` into either the cache key or argv, and two
    // `since_baseline` calls that differ only in a caller-supplied `days`/
    // `since` still coalesce onto the same cache entry.
    let (days, since) = if since_baseline {
        (None, None)
    } else if since.is_some() {
        (None, since)
    } else {
        (Some(days.unwrap_or(7).clamp(1, 365)), since)
    };
    let key: CacheKey = (env.clone(), days, since.clone(), to.clone(), since_baseline);

    // Hold the lock across the fetch so concurrent callers coalesce onto one
    // CLI run rather than each spawning their own.
    let mut guard = cache().lock().await;
    if !refresh.unwrap_or(false) {
        if let Some(c) = guard.as_ref() {
            if c.key == key && c.fetched.elapsed() < CACHE_TTL {
                return Ok(c.value.clone());
            }
        }
    }

    let wickd = find_wickd_binary()?.ok_or_else(|| {
        "wickd CLI not found — install it (cargo install) to see account performance".to_string()
    })?;

    let args = build_glance_args(&env, days, since.as_deref(), to.as_deref(), since_baseline);

    let output = tokio::time::timeout(
        GLANCE_TIMEOUT,
        tokio::process::Command::new(&wickd).args(&args).output(),
    )
    .await
    .map_err(|_| "account fetch timed out".to_string())?
    .map_err(|e| format!("running wickd trade glance: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).map_err(|_| {
        // The CLI emits JSON on both paths; anything else means it never ran.
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = if stderr.trim().is_empty() { stdout } else { stderr };
        format!("unexpected wickd output: {}", detail.chars().take(200).collect::<String>())
    })?;

    if let Some(err) = value.get("error") {
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
        return Err(msg.to_string());
    }

    let glance: AccountsGlance =
        serde_json::from_value(value).map_err(|e| format!("unexpected glance shape: {e}"))?;

    *guard = Some(Cached { key, value: glance.clone(), fetched: Instant::now() });
    Ok(glance)
}

/// An account name is safe to pass as a CLI argument only if it matches the
/// vault's own naming rule. Mirrors `vault_store::validate_account_name`, which
/// is not importable from the app crate — the point is to reject anything
/// exotic before it becomes argv, not to perfectly reproduce that function.
fn valid_account_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Build the `wickd trade history` argv for one account (D10). Pure so the
/// exact argv per mode is unit-tested without shelling out.
fn build_history_args(
    env: &str,
    account: &str,
    since: Option<&str>,
    to: Option<&str>,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "trade".into(),
        "history".into(),
        "--env".into(),
        env.into(),
        "--account".into(),
        account.into(),
    ];
    if let Some(s) = since {
        args.extend(["--since".to_string(), s.to_string()]);
    }
    if let Some(t) = to {
        args.extend(["--to".to_string(), t.to_string()]);
    }
    args
}

/// Closed-trade history for one account, with entry/exit detail, since its
/// experiment start (or `since`), optionally closed at `to`. Shells out to
/// `wickd trade history`.
///
/// Returned as raw JSON: the per-trade shape is rich (entry, exit, blended
/// flag, duration) and the frontend renders it directly rather than round-trip
/// it through a typed mirror that would need updating in lockstep. This is the
/// drill-down behind an account tile — user-triggered, never on the boot path,
/// and it reaches OANDA so it is not offline.
#[tauri::command]
pub async fn account_history(
    account: String,
    since: Option<String>,
    to: Option<String>,
    env: Option<String>,
) -> Result<serde_json::Value, String> {
    if !valid_account_name(&account) {
        return Err(format!("invalid account name '{account}'"));
    }
    let env = match env.as_deref().unwrap_or("practice") {
        e @ ("practice" | "live") => e.to_string(),
        other => return Err(format!("unknown environment '{other}'")),
    };
    let since = match since {
        Some(s) => Some(validate_rfc3339("since", &s)?),
        None => None,
    };
    let to = match to {
        Some(t) => Some(validate_rfc3339("to", &t)?),
        None => None,
    };

    let wickd = find_wickd_binary()?.ok_or_else(|| {
        "wickd CLI not found — install it (cargo install) to see trade history".to_string()
    })?;

    let args = build_history_args(&env, &account, since.as_deref(), to.as_deref());

    let output = tokio::time::timeout(
        GLANCE_TIMEOUT,
        tokio::process::Command::new(&wickd).args(&args).output(),
    )
    .await
    .map_err(|_| "history fetch timed out".to_string())?
    .map_err(|e| format!("running wickd trade history: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).map_err(|_| {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = if stderr.trim().is_empty() { stdout } else { stderr };
        format!("unexpected wickd output: {}", detail.chars().take(200).collect::<String>())
    })?;

    if let Some(err) = value.get("error") {
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
        return Err(msg.to_string());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY_MIDNIGHT: &str = "2026-08-25T00:00:00Z";
    const CUSTOM_SINCE: &str = "2026-08-01T00:00:00Z";
    const CUSTOM_TO: &str = "2026-08-24T00:00:00Z";

    // --- build_glance_args: exact argv per window mode (AC4) ---

    #[test]
    fn glance_args_today_mode_uses_since_no_days() {
        // The frontend's "Today" passes an explicit `since` (local midnight)
        // and no `days`; `days` must already be None by the time it reaches
        // the arg builder (the command normalizes this before calling it).
        let args =
            build_glance_args("practice", None, Some(TODAY_MIDNIGHT), None, false);
        assert_eq!(
            args,
            vec![
                "trade", "glance", "--env", "practice", "--since", TODAY_MIDNIGHT,
            ]
        );
    }

    #[test]
    fn glance_args_days_mode() {
        let args = build_glance_args("practice", Some(7), None, None, false);
        assert_eq!(
            args,
            vec!["trade", "glance", "--env", "practice", "--days", "7"]
        );
    }

    #[test]
    fn glance_args_since_and_to_compose() {
        let args = build_glance_args(
            "practice",
            None,
            Some(CUSTOM_SINCE),
            Some(CUSTOM_TO),
            false,
        );
        assert_eq!(
            args,
            vec![
                "trade", "glance", "--env", "practice", "--since", CUSTOM_SINCE, "--to",
                CUSTOM_TO,
            ]
        );
    }

    #[test]
    fn glance_args_since_baseline_mode_emits_only_the_flag() {
        // days/since are expected already normalized to None by the caller
        // (AC1: since_baseline ignores them) — the builder must not emit
        // --since or --days alongside --since-baseline, mirroring the CLI's
        // conflicts_with.
        let args = build_glance_args("practice", None, None, None, true);
        assert_eq!(
            args,
            vec!["trade", "glance", "--env", "practice", "--since-baseline"]
        );
    }

    #[test]
    fn glance_args_since_baseline_composes_with_to() {
        // AGT-1129 D4: --to is NOT in --since-baseline's conflicts list.
        let args = build_glance_args("practice", None, None, Some(CUSTOM_TO), true);
        assert_eq!(
            args,
            vec![
                "trade",
                "glance",
                "--env",
                "practice",
                "--since-baseline",
                "--to",
                CUSTOM_TO,
            ]
        );
    }

    #[test]
    fn glance_args_live_env_is_passed_through() {
        let args = build_glance_args("live", Some(7), None, None, false);
        assert_eq!(args[..4], ["trade", "glance", "--env", "live"]);
    }

    // --- build_history_args ---

    #[test]
    fn history_args_no_window() {
        let args = build_history_args("practice", "h004", None, None);
        assert_eq!(
            args,
            vec!["trade", "history", "--env", "practice", "--account", "h004"]
        );
    }

    #[test]
    fn history_args_since_and_to() {
        let args =
            build_history_args("practice", "h004", Some(CUSTOM_SINCE), Some(CUSTOM_TO));
        assert_eq!(
            args,
            vec![
                "trade", "history", "--env", "practice", "--account", "h004", "--since",
                CUSTOM_SINCE, "--to", CUSTOM_TO,
            ]
        );
    }

    #[test]
    fn history_args_to_only() {
        // D8: the "baseline" window passes neither since nor to; a "range"
        // window passes both. --to alone (e.g. a range whose since defaults
        // to the account's baseline) must still land after --account with no
        // --since in between.
        let args = build_history_args("practice", "h004", None, Some(CUSTOM_TO));
        assert_eq!(
            args,
            vec![
                "trade", "history", "--env", "practice", "--account", "h004", "--to",
                CUSTOM_TO,
            ]
        );
    }

    // --- validate_rfc3339: malformed input is rejected before argv (AC1, D10) ---

    #[test]
    fn validate_rfc3339_accepts_a_real_instant() {
        assert!(validate_rfc3339("to", CUSTOM_TO).is_ok());
    }

    #[test]
    fn validate_rfc3339_rejects_malformed_input_naming_the_field_and_value() {
        let err = validate_rfc3339("to", "not-a-date").unwrap_err();
        assert!(err.contains("invalid to"), "error should name the field: {err}");
        assert!(err.contains("not-a-date"), "error should name the input: {err}");
    }

    #[test]
    fn validate_rfc3339_rejects_a_bare_date_like_since_does_today() {
        // Unlike the CLI's parse_baseline_date, the Tauri boundary only
        // accepts full RFC3339 (matches the existing `since` validation this
        // mirrors for `to`) — a bare ISO date is not RFC3339 and is rejected.
        assert!(validate_rfc3339("to", "2026-08-24").is_err());
    }

    // --- CacheKey discriminates all five components (D9, AC3) ---

    #[test]
    fn cache_key_discriminates_env() {
        let a: CacheKey = ("practice".into(), Some(7), None, None, false);
        let b: CacheKey = ("live".into(), Some(7), None, None, false);
        assert_ne!(a, b);
    }

    #[test]
    fn cache_key_discriminates_days() {
        let a: CacheKey = ("practice".into(), Some(7), None, None, false);
        let b: CacheKey = ("practice".into(), Some(30), None, None, false);
        assert_ne!(a, b);
    }

    #[test]
    fn cache_key_discriminates_since() {
        let a: CacheKey = ("practice".into(), None, Some(TODAY_MIDNIGHT.into()), None, false);
        let b: CacheKey = ("practice".into(), None, Some(CUSTOM_SINCE.into()), None, false);
        assert_ne!(a, b);
    }

    #[test]
    fn cache_key_discriminates_to() {
        let a: CacheKey =
            ("practice".into(), None, Some(CUSTOM_SINCE.into()), Some(CUSTOM_TO.into()), false);
        let b: CacheKey = ("practice".into(), None, Some(CUSTOM_SINCE.into()), None, false);
        assert_ne!(a, b);
    }

    #[test]
    fn cache_key_discriminates_since_baseline() {
        // Same env/days/since/to — only the since_baseline bool differs. A
        // since_baseline glance must never be served from a non-baseline
        // glance's cache entry, and vice versa.
        let a: CacheKey = ("practice".into(), None, None, None, true);
        let b: CacheKey = ("practice".into(), None, None, None, false);
        assert_ne!(a, b);
    }

    #[test]
    fn cache_key_equal_when_every_component_matches() {
        let a: CacheKey =
            ("practice".into(), Some(7), None, Some(CUSTOM_TO.into()), false);
        let b: CacheKey =
            ("practice".into(), Some(7), None, Some(CUSTOM_TO.into()), false);
        assert_eq!(a, b);
    }

    // --- deserialization of the real CLI shapes (widened Option types) ---

    #[test]
    fn accounts_glance_deserializes_the_since_baseline_shape() {
        // Under --since-baseline the top-level `since`/`days` are null, and a
        // no-baseline row nulls its numeric fields with a note (D3) — this is
        // the exact shape observed from the live CLI binary. Before AGT-1130
        // `since` was a non-Option `String`, so this payload failed to
        // deserialize entirely.
        let raw = serde_json::json!({
            "environment": "practice",
            "days": null,
            "since": null,
            "to": "2026-08-25T02:02:25.857049+00:00",
            "generated_at": "2026-08-25T02:02:25.857049+00:00",
            "count": 1,
            "accounts": [{
                "account": "boc-rba",
                "names": ["boc-rba", "tf-m30"],
                "account_id": "101-001-26151603-005",
                "alias": "Retired - do not trade",
                "open_positions": [],
                "currency": "USD",
                "nav": "100048.1138",
                "balance": "100048.1138",
                "unrealized_pl": "0.0000",
                "open_trade_count": 0,
                "realized": null,
                "trades": null,
                "wins": null,
                "losses": null,
                "win_rate": null,
                "window_start": null,
                "window_source": "baseline",
                "note": "no baseline recorded",
                "error": null
            }]
        });
        let glance: AccountsGlance = serde_json::from_value(raw).expect("should deserialize");
        assert_eq!(glance.since, None);
        assert_eq!(glance.days, None);
        let row = &glance.accounts[0];
        assert_eq!(row.realized, None);
        assert_eq!(row.trades, None);
        assert_eq!(row.window_start, None);
        assert_eq!(row.window_source.as_deref(), Some("baseline"));
        assert_eq!(row.note.as_deref(), Some("no baseline recorded"));
    }

    #[test]
    fn accounts_glance_deserializes_the_ordinary_days_shape() {
        let raw = serde_json::json!({
            "environment": "practice",
            "days": 7,
            "since": "2026-08-18T02:02:25.857049+00:00",
            "to": "2026-08-25T02:02:25.857049+00:00",
            "generated_at": "2026-08-25T02:02:25.857049+00:00",
            "count": 1,
            "accounts": [{
                "account": "h004",
                "names": ["h004", "default"],
                "account_id": "101-001-26151603-001",
                "alias": "Scratch - do not trade",
                "open_positions": [],
                "currency": "USD",
                "nav": "9999.9965",
                "balance": "9999.9965",
                "unrealized_pl": "0.0000",
                "open_trade_count": 0,
                "realized": "0",
                "trades": 0,
                "wins": 0,
                "losses": 0,
                "win_rate": null,
                "window_start": "2026-08-18T02:02:25.857049+00:00",
                "window_source": "days",
                "note": null,
                "error": null
            }]
        });
        let glance: AccountsGlance = serde_json::from_value(raw).expect("should deserialize");
        assert_eq!(glance.since.as_deref(), Some("2026-08-18T02:02:25.857049+00:00"));
        assert_eq!(glance.days, Some(7));
        let row = &glance.accounts[0];
        assert_eq!(row.trades, Some(0));
        assert_eq!(row.window_source.as_deref(), Some("days"));
        assert_eq!(row.note, None);
    }
}
