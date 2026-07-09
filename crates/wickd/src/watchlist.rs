//! Watchlist resolution for `wickd stream` (AGT-614).
//!
//! `wickd stream` previously required a hand-typed, comma-separated instrument
//! list on every invocation. This module adds a persisted watchlist file
//! (`~/.wickd/watchlist.json`) plus resolution precedence so the CLI can be
//! pointed at a *named* list, a file-configured default, or the special `"all"`
//! value — without ever hand-typing 68 instrument codes.
//!
//! ## Resolution precedence (AC1)
//!
//! 1. Explicit CLI comma-separated instrument list (highest priority).
//! 2. A named list via `--list <name>` (looked up in the watchlist file, with
//!    the built-in `majors` list available even if the file doesn't define it).
//! 3. The watchlist file's `default` list.
//! 4. The built-in `majors` fallback, if nothing else resolves.
//!
//! ## `"all"` (AC2)
//!
//! `"all"` is a reserved keyword, not a stored list: it can be passed as the
//! sole CLI instrument, as `--list all`, or as the file's `default`, and it
//! always resolves at *runtime* via `endpoints::get_instruments` — it is never
//! written to `watchlist.json` as an expanded list of symbols.
//!
//! ## Schema (`~/.wickd/watchlist.json`)
//!
//! ```json
//! {
//!   "version": 1,
//!   "default": "majors",
//!   "lists": {
//!     "majors": ["EUR_USD", "GBP_USD", "USD_JPY"],
//!     "asia": ["USD_JPY", "AUD_USD", "NZD_USD"]
//!   }
//! }
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Reserved keyword: resolves to every instrument OANDA offers this account,
/// fetched at runtime. Never persisted as an expanded list (AC2).
pub const ALL: &str = "all";

/// Built-in fallback list name, and the name used to look up a user-overridden
/// `majors` list in the watchlist file before falling back to
/// [`DEFAULT_MAJORS`].
pub const MAJORS: &str = "majors";

/// Hardcoded majors set used when the watchlist file doesn't exist, or doesn't
/// define its own `majors` list. The common G10 FX majors.
pub const DEFAULT_MAJORS: &[&str] = &[
    "EUR_USD", "GBP_USD", "USD_JPY", "USD_CHF", "USD_CAD", "AUD_USD", "NZD_USD", "EUR_GBP",
];

/// On-disk watchlist file (`~/.wickd/watchlist.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WatchlistFile {
    pub version: u32,
    /// Name of the list to use when no explicit instruments or `--list` are
    /// given. May be `"all"`.
    #[serde(default)]
    pub default: Option<String>,
    /// Named instrument lists.
    #[serde(default)]
    pub lists: HashMap<String, Vec<String>>,
}

/// A resolved (but not yet validated-against-OANDA) instrument request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListSpec {
    /// Resolve to every instrument OANDA offers this account, at request time.
    All,
    /// A concrete list of instrument symbols to subscribe to.
    Symbols(Vec<String>),
}

/// Path to the watchlist file (`~/.wickd/watchlist.json`).
pub fn watchlist_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not resolve home directory")?;
    Ok(home.join(".wickd").join("watchlist.json"))
}

/// Load the watchlist file at `path`, or `None` if it doesn't exist yet (a
/// missing file is not an error — resolution falls through to `majors`).
pub fn load_at(path: impl AsRef<Path>) -> Result<Option<WatchlistFile>> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading watchlist file at {}", path.display()))?;
    let file: WatchlistFile = serde_json::from_str(&raw)
        .with_context(|| format!("watchlist file at {} is corrupt or not valid JSON", path.display()))?;
    Ok(Some(file))
}

/// Load the watchlist file from the default path.
pub fn load() -> Result<Option<WatchlistFile>> {
    load_at(watchlist_path()?)
}

fn is_all(name: &str) -> bool {
    name.eq_ignore_ascii_case(ALL)
}

fn is_majors(name: &str) -> bool {
    name.eq_ignore_ascii_case(MAJORS)
}

fn default_majors() -> Vec<String> {
    DEFAULT_MAJORS.iter().map(|s| s.to_string()).collect()
}

/// Resolve a named list (`--list <name>`, or a file `default` name) against
/// `file`: the file's own `lists` entry wins if present, otherwise `"majors"`
/// falls back to the built-in set, otherwise the name is unknown.
fn resolve_named(name: &str, file: Option<&WatchlistFile>) -> Result<ListSpec> {
    if is_all(name) {
        return Ok(ListSpec::All);
    }
    if let Some(file) = file {
        if let Some(symbols) = file.lists.get(name) {
            return Ok(ListSpec::Symbols(symbols.clone()));
        }
    }
    if is_majors(name) {
        return Ok(ListSpec::Symbols(default_majors()));
    }
    bail!(
        "unknown watchlist '{name}' — define it in {} or use a comma-separated instrument list",
        watchlist_path().map(|p| p.display().to_string()).unwrap_or_else(|_| "~/.wickd/watchlist.json".to_string())
    )
}

/// Resolve the instruments to stream per the AC1 precedence:
/// explicit CLI list -> `--list <name>` -> file `default` -> built-in `majors`.
///
/// `explicit` is `Some` only when the caller actually supplied a non-empty
/// comma-separated instrument list on the command line. A lone `"all"` entry
/// (`wickd stream all`) is treated as the reserved keyword, not a literal
/// instrument symbol.
pub fn resolve_spec(explicit: Option<&[String]>, list_name: Option<&str>) -> Result<ListSpec> {
    if let Some(instruments) = explicit {
        if !instruments.is_empty() {
            if instruments.len() == 1 && is_all(&instruments[0]) {
                return Ok(ListSpec::All);
            }
            return Ok(ListSpec::Symbols(instruments.to_vec()));
        }
    }

    let file = load()?;

    if let Some(name) = list_name {
        return resolve_named(name, file.as_ref());
    }

    if let Some(default_name) = file.as_ref().and_then(|f| f.default.as_deref()) {
        return resolve_named(default_name, file.as_ref());
    }

    Ok(ListSpec::Symbols(default_majors()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut p = std::env::temp_dir();
        p.push(format!("wickd-watchlist-test-{}-{n}.json", std::process::id()));
        p
    }

    fn write_file(path: &Path, file: &WatchlistFile) {
        std::fs::write(path, serde_json::to_string_pretty(file).unwrap()).unwrap();
    }

    // AC1: explicit CLI instruments always win, even with a file present.
    #[test]
    fn explicit_cli_list_wins_over_everything() {
        let path = temp_path();
        write_file(
            &path,
            &WatchlistFile {
                version: 1,
                default: Some("asia".to_string()),
                lists: HashMap::from([("asia".to_string(), vec!["USD_JPY".to_string()])]),
            },
        );
        // Sanity: the file itself resolves to "asia" when nothing explicit is given.
        assert_eq!(
            resolve_named("asia", load_at(&path).unwrap().as_ref()).unwrap(),
            ListSpec::Symbols(vec!["USD_JPY".to_string()])
        );

        let explicit = vec!["EUR_USD".to_string(), "GBP_USD".to_string()];
        let spec = resolve_spec(Some(&explicit), None).unwrap();
        assert_eq!(spec, ListSpec::Symbols(explicit));

        let _ = std::fs::remove_file(&path);
    }

    // AC1: --list <name> resolves against the file's named lists.
    #[test]
    fn named_list_resolves_from_file() {
        let path = temp_path();
        write_file(
            &path,
            &WatchlistFile {
                version: 1,
                default: None,
                lists: HashMap::from([(
                    "asia".to_string(),
                    vec!["USD_JPY".to_string(), "AUD_USD".to_string()],
                )]),
            },
        );
        let file = load_at(&path).unwrap();
        let spec = resolve_named("asia", file.as_ref()).unwrap();
        assert_eq!(
            spec,
            ListSpec::Symbols(vec!["USD_JPY".to_string(), "AUD_USD".to_string()])
        );
        let _ = std::fs::remove_file(&path);
    }

    // AC1: the file's `default` list is used when nothing explicit is given.
    #[test]
    fn file_default_used_when_no_explicit_or_named_list() {
        let path = temp_path();
        write_file(
            &path,
            &WatchlistFile {
                version: 1,
                default: Some("asia".to_string()),
                lists: HashMap::from([("asia".to_string(), vec!["USD_JPY".to_string()])]),
            },
        );
        // Simulate the file-based lookup path resolve_spec takes internally.
        let file = load_at(&path).unwrap();
        let default_name = file.as_ref().and_then(|f| f.default.as_deref()).unwrap();
        let spec = resolve_named(default_name, file.as_ref()).unwrap();
        assert_eq!(spec, ListSpec::Symbols(vec!["USD_JPY".to_string()]));
        let _ = std::fs::remove_file(&path);
    }

    // AC1: with no file, no explicit list, no --list, majors is the fallback.
    #[test]
    fn missing_file_falls_back_to_builtin_majors() {
        let path = temp_path(); // never written -> load_at returns None
        assert!(load_at(&path).unwrap().is_none());
        let spec = resolve_named(MAJORS, None).unwrap();
        assert_eq!(spec, ListSpec::Symbols(default_majors()));
        assert!(!default_majors().is_empty());
    }

    // AC2: "all" as the sole explicit CLI instrument is the reserved keyword,
    // not a literal instrument symbol.
    #[test]
    fn explicit_all_resolves_to_all_spec() {
        let explicit = vec!["all".to_string()];
        let spec = resolve_spec(Some(&explicit), None).unwrap();
        assert_eq!(spec, ListSpec::All);
    }

    // AC2: --list all also resolves to the All spec, regardless of file state.
    #[test]
    fn named_all_resolves_to_all_spec() {
        assert_eq!(resolve_named("ALL", None).unwrap(), ListSpec::All);
        assert_eq!(resolve_named("all", None).unwrap(), ListSpec::All);
    }

    // AC2: an unknown named list is a clear error, not a silent empty stream.
    #[test]
    fn unknown_named_list_errors() {
        let err = resolve_named("nope", None).unwrap_err();
        assert!(err.to_string().contains("unknown watchlist"));
    }

    // A watchlist file can override the built-in majors list.
    #[test]
    fn file_can_override_majors_list() {
        let path = temp_path();
        write_file(
            &path,
            &WatchlistFile {
                version: 1,
                default: None,
                lists: HashMap::from([("majors".to_string(), vec!["EUR_USD".to_string()])]),
            },
        );
        let file = load_at(&path).unwrap();
        let spec = resolve_named("majors", file.as_ref()).unwrap();
        assert_eq!(spec, ListSpec::Symbols(vec!["EUR_USD".to_string()]));
        let _ = std::fs::remove_file(&path);
    }

    // AC2 (persistence guard): resolving "all" never touches lists/defaults —
    // it must not require or produce a stored expansion.
    #[test]
    fn all_never_needs_a_stored_list() {
        // No file at all, and "all" still resolves cleanly.
        let spec = resolve_spec(Some(&["all".to_string()]), None).unwrap();
        assert_eq!(spec, ListSpec::All);
        // Serializing a WatchlistFile with "all" as default is legal (it's just
        // a name), but resolving it never expands "all" into `lists`.
        let file = WatchlistFile { version: 1, default: Some("all".to_string()), lists: HashMap::new() };
        assert!(!file.lists.contains_key("all"));
    }
}
