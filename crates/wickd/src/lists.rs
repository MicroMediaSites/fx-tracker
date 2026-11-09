//! Named symbol lists (`~/.wickd/lists.json`, wickd-lists AGT-2391).
//!
//! A list is an ordered set of stocks and/or forex pairs, each with an
//! optional note (why it's there). Lists are separate from the TUI's general
//! watchlist (`tui.json`), from the in-play screen, and from the forex
//! streamer's `~/.wickd/watchlist.json`. They are the agent-facing surface:
//! `wickd list …` writes them; the TUI shows them as watchlist sections and
//! can remove entries.
//!
//! ## Writers
//!
//! The CLI (often an agent) and the TUI both write this file. Every change is
//! a read-modify-write under an exclusive `flock` on `lists.json.lock`,
//! finished with a tmp-file + rename, so concurrent writers never lose each
//! other's changes and a crash never leaves a truncated file.
//!
//! ```json
//! {
//!   "version": 1,
//!   "lists": {
//!     "tomorrow": [
//!       {"symbol": "NVDA", "note": "beat; watch 185", "added": "2026-11-09T03:00:00Z"}
//!     ]
//!   }
//! }
//! ```

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A problem with the caller's input (bad name, unknown symbol, no such
/// list) — exit code 4 — as opposed to a broken store or I/O failure.
#[derive(Debug)]
pub struct ValidationError(pub String);

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ValidationError {}

fn invalid(msg: String) -> anyhow::Error {
    ValidationError(msg).into()
}

/// The error for a list that doesn't exist (with the next step).
pub fn no_list(name: &str) -> anyhow::Error {
    invalid(format!("no list '{name}' (see `wickd list ls`)"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub symbol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub added: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ListsFile {
    #[serde(default = "version")]
    pub version: u32,
    #[serde(default)]
    pub lists: BTreeMap<String, Vec<Entry>>,
}

fn version() -> u32 {
    1
}

pub fn lists_path() -> Result<PathBuf> {
    Ok(wickd_core::paths::wickd_data_home()
        .map_err(|e| anyhow!(e))?
        .join("lists.json"))
}

/// Read the lists (an absent file is no lists).
pub fn load_at(path: &Path) -> Result<ListsFile> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("{} is not valid lists JSON", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ListsFile {
            version: 1,
            ..Default::default()
        }),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Apply `change` to the lists under an exclusive lock and save atomically.
/// Returns whatever `change` returns.
pub fn update_at<T>(path: &Path, change: impl FnOnce(&mut ListsFile) -> Result<T>) -> Result<T> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("json.lock"))
        .context("opening the lists lock file")?;
    let _guard = Flock::exclusive(&lock)?;
    let mut file = load_at(path)?;
    let out = change(&mut file)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&file)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(out)
}

/// Exclusive advisory lock, released on drop.
struct Flock<'a>(&'a std::fs::File);

impl<'a> Flock<'a> {
    fn exclusive(f: &'a std::fs::File) -> Result<Self> {
        use std::os::unix::io::AsRawFd;
        // SAFETY: flock on a file descriptor we own for the guard's lifetime.
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
            bail!("locking lists.json: {}", std::io::Error::last_os_error());
        }
        Ok(Self(f))
    }
}

impl Drop for Flock<'_> {
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd;
        // SAFETY: as above; unlocking is best-effort (closing the fd also unlocks).
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

/// List names: lowercase letters, digits, `-` and `_` (they're typed in
/// commands and shown as section titles).
pub fn validate_name(name: &str) -> Result<String> {
    let n = name.trim().to_ascii_lowercase();
    if n.is_empty()
        || n.len() > 32
        || !n
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(invalid(format!(
            "invalid list name '{name}': use a-z, 0-9, - or _ (max 32)"
        )));
    }
    Ok(n)
}

/// Normalise a typed symbol: upper-case, and `EUR/USD` → `EUR_USD`.
pub fn normalize_symbol(s: &str) -> String {
    s.trim().to_ascii_uppercase().replace('/', "_")
}

/// Check `symbols` against the known stock + pair symbols (`None` = no
/// symbol cache yet: accept, the caller reports them as unverified).
pub fn check_known(symbols: &[String], known: Option<&HashSet<String>>) -> Result<()> {
    let Some(known) = known else {
        return Ok(());
    };
    let unknown: Vec<&str> = symbols
        .iter()
        .filter(|s| !known.contains(*s))
        .map(String::as_str)
        .collect();
    if !unknown.is_empty() {
        return Err(invalid(format!(
            "unknown symbol(s): {} (stocks use their ticker, pairs OANDA names like EUR_USD)",
            unknown.join(", ")
        )));
    }
    Ok(())
}

/// Add `symbols` to `list` (created if missing). An existing entry keeps its
/// place; a given note replaces its note. Returns how many were new.
pub fn add(file: &mut ListsFile, list: &str, symbols: &[String], note: Option<&str>, now: DateTime<Utc>) -> usize {
    let entries = file.lists.entry(list.to_string()).or_default();
    let note = note.map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
    let mut added = 0;
    for sym in symbols {
        match entries.iter_mut().find(|e| e.symbol == *sym) {
            Some(e) => {
                if note.is_some() {
                    e.note = note.clone();
                }
            }
            None => {
                entries.push(Entry {
                    symbol: sym.clone(),
                    note: note.clone(),
                    added: now,
                });
                added += 1;
            }
        }
    }
    added
}

/// Remove `symbols` from `list`; returns the ones that were there. A list
/// left empty is kept (it's still a named list) until cleared/deleted.
pub fn remove(file: &mut ListsFile, list: &str, symbols: &[String]) -> Result<Vec<String>> {
    let entries = file
        .lists
        .get_mut(list)
        .ok_or_else(|| no_list(list))?;
    let before: Vec<String> = entries.iter().map(|e| e.symbol.clone()).collect();
    entries.retain(|e| !symbols.contains(&e.symbol));
    Ok(before.into_iter().filter(|s| symbols.contains(s)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        "2026-11-09T03:00:00Z".parse().unwrap()
    }

    #[test]
    fn add_keeps_order_updates_notes_and_remove_reports() {
        let mut f = ListsFile::default();
        let n = add(&mut f, "tomorrow", &["NVDA".into(), "AMD".into()], Some("earnings"), now());
        assert_eq!(n, 2);
        // Re-adding NVDA with a new note: same place, new note, not counted.
        let n = add(&mut f, "tomorrow", &["NVDA".into(), "EUR_USD".into()], Some("gap up"), now());
        assert_eq!(n, 1);
        let syms: Vec<_> = f.lists["tomorrow"].iter().map(|e| e.symbol.as_str()).collect();
        assert_eq!(syms, ["NVDA", "AMD", "EUR_USD"]);
        assert_eq!(f.lists["tomorrow"][0].note.as_deref(), Some("gap up"));
        assert_eq!(f.lists["tomorrow"][1].note.as_deref(), Some("earnings"));
        // No note given: the existing note stays.
        add(&mut f, "tomorrow", &["AMD".into()], None, now());
        assert_eq!(f.lists["tomorrow"][1].note.as_deref(), Some("earnings"));
        let gone = remove(&mut f, "tomorrow", &["AMD".into(), "TSLA".into()]).unwrap();
        assert_eq!(gone, ["AMD"]);
        assert!(remove(&mut f, "nope", &["X".into()]).is_err());
    }

    #[test]
    fn names_and_symbols_are_normalised_and_checked() {
        assert_eq!(validate_name(" Tomorrow ").unwrap(), "tomorrow");
        assert!(validate_name("my list").is_err());
        assert!(validate_name("").is_err());
        assert_eq!(normalize_symbol(" eur/usd "), "EUR_USD");
        assert_eq!(normalize_symbol("brk.b"), "BRK.B");
        let known: HashSet<String> = ["NVDA", "EUR_USD"].iter().map(|s| s.to_string()).collect();
        assert!(check_known(&["NVDA".into(), "EUR_USD".into()], Some(&known)).is_ok());
        let e = check_known(&["NVDA".into(), "NVDAA".into()], Some(&known)).unwrap_err();
        assert!(e.to_string().contains("NVDAA"));
        assert!(e.downcast_ref::<ValidationError>().is_some());
        // A broken store is not a validation error.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lists.json");
        std::fs::write(&path, "{ invalid json").unwrap();
        assert!(load_at(&path).unwrap_err().downcast_ref::<ValidationError>().is_none());
        assert!(check_known(&["ANY".into()], None).is_ok());
    }

    #[test]
    fn concurrent_writers_never_lose_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lists.json");
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for j in 0..10 {
                        update_at(&path, |f| {
                            add(f, "t", &[format!("S{i}_{j}")], None, now());
                            Ok(())
                        })
                        .unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(load_at(&path).unwrap().lists["t"].len(), 80);
    }

    #[test]
    fn missing_file_is_empty_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lists.json");
        assert!(load_at(&path).unwrap().lists.is_empty());
        update_at(&path, |f| {
            add(f, "tomorrow", &["SPY".into()], Some("open drive"), now());
            Ok(())
        })
        .unwrap();
        let f = load_at(&path).unwrap();
        assert_eq!(f.lists["tomorrow"][0].note.as_deref(), Some("open drive"));
    }
}
