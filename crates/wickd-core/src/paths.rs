//! Central resolution of the wickd data home (`~/.wickd`).
//!
//! Historically every store resolved `dirs::home_dir().join(".wickd")` on its
//! own. This module is the shared resolver for *new* code (the strategy store,
//! the event calendars, the desktop app's local store); existing per-store
//! `*_path()` helpers keep working unchanged.
//!
//! ## `WICKD_HOME`
//!
//! Tests and smoke runs can point the data home somewhere else by setting the
//! `WICKD_HOME` environment variable to a directory (the literal data home —
//! no `.wickd` suffix is appended). This must never be set for the live
//! daemons; it exists so agents/tests can exercise store paths against a temp
//! dir without touching the real `~/.wickd`.

use std::path::PathBuf;

/// Environment variable overriding the data home (used by tests/smoke runs).
pub const WICKD_HOME_ENV: &str = "WICKD_HOME";

/// Resolve the wickd data home: `$WICKD_HOME` when set and non-empty, else
/// `~/.wickd`.
pub fn wickd_data_home() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os(WICKD_HOME_ENV) {
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }
    dirs::home_dir()
        .map(|h| h.join(".wickd"))
        .ok_or_else(|| "could not resolve home directory".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: no test mutates WICKD_HOME via std::env::set_var — env is process
    // global and tests run in parallel. The override branch is covered by the
    // strategy-store integration tests, which spawn with an explicit root.

    #[test]
    fn default_data_home_ends_with_dot_wickd() {
        // Only run the default branch when the override is absent (it is in CI).
        if std::env::var_os(WICKD_HOME_ENV).is_none() {
            let home = wickd_data_home().unwrap();
            assert!(home.ends_with(".wickd"), "got {}", home.display());
        }
    }
}
