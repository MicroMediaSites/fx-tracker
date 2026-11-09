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
//!     "mon-11-9": [
//!       {"symbol": "NET", "why": "earnings: sales +39%, +10% Fri",
//!        "call": "continues", "constraints": "stop below Fri low",
//!        "signal": "holds Fri close after 8:00 on volume",
//!        "added": "2026-11-09T03:00:00Z"}
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

/// One symbol on a list, with the plan for it (all optional; wickd-learn
/// D-13): why it's listed, the call, the constraints, and the signal to act
/// on. Files written before these fields used a single `note`, which reads
/// as `why`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub symbol: String,
    #[serde(default, alias = "note", skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
    pub added: DateTime<Utc>,
}

/// Plan fields to write onto entries: `Some("")` clears a field, `None`
/// leaves it as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub why: Option<String>,
    pub call: Option<String>,
    pub constraints: Option<String>,
    pub signal: Option<String>,
}

impl Plan {
    /// Just a `why` (the old single note).
    pub fn why(s: &str) -> Self {
        Self {
            why: Some(s.to_string()),
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.why.is_none() && self.call.is_none() && self.constraints.is_none() && self.signal.is_none()
    }

    fn apply(&self, e: &mut Entry) {
        let set = |field: &mut Option<String>, v: &Option<String>| {
            if let Some(v) = v {
                let v = v.trim();
                *field = (!v.is_empty()).then(|| v.to_string());
            }
        };
        set(&mut e.why, &self.why);
        set(&mut e.call, &self.call);
        set(&mut e.constraints, &self.constraints);
        set(&mut e.signal, &self.signal);
    }
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

/// What an add changed: symbols that are new, and existing ones whose plan
/// fields were written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Added {
    pub added: Vec<String>,
    pub updated: Vec<String>,
}

/// Add `symbols` to `list` (created if missing). An existing entry keeps its
/// place; the given plan fields replace its fields (others are kept).
pub fn add(file: &mut ListsFile, list: &str, symbols: &[String], plan: &Plan, now: DateTime<Utc>) -> Added {
    let entries = file.lists.entry(list.to_string()).or_default();
    let mut out = Added::default();
    for sym in symbols {
        match entries.iter_mut().find(|e| e.symbol == *sym) {
            Some(e) => {
                if !plan.is_empty() {
                    plan.apply(e);
                    out.updated.push(sym.clone());
                }
            }
            None => {
                let mut e = Entry {
                    symbol: sym.clone(),
                    why: None,
                    call: None,
                    constraints: None,
                    signal: None,
                    added: now,
                };
                plan.apply(&mut e);
                entries.push(e);
                out.added.push(sym.clone());
            }
        }
    }
    out
}

/// Write plan fields onto one existing entry (e.g. the call, before the
/// open). The symbol must already be on the list.
pub fn set(file: &mut ListsFile, list: &str, symbol: &str, plan: &Plan) -> Result<Entry> {
    if plan.is_empty() {
        return Err(invalid(
            "nothing to set: give --why, --call, --constraints and/or --signal".into(),
        ));
    }
    let entries = file.lists.get_mut(list).ok_or_else(|| no_list(list))?;
    let e = entries
        .iter_mut()
        .find(|e| e.symbol == symbol)
        .ok_or_else(|| {
            invalid(format!(
                "{symbol} isn't on list '{list}' (add it with `wickd list add {list} {symbol}`)"
            ))
        })?;
    plan.apply(e);
    Ok(e.clone())
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
    fn add_keeps_order_updates_fields_and_remove_reports() {
        let mut f = ListsFile::default();
        let r = add(&mut f, "tomorrow", &["NVDA".into(), "AMD".into()], &Plan::why("earnings"), now());
        assert_eq!(r.added, ["NVDA", "AMD"]);
        // Re-adding NVDA with a new why: same place, updated, not new.
        let r = add(&mut f, "tomorrow", &["NVDA".into(), "EUR_USD".into()], &Plan::why("gap up"), now());
        assert_eq!((r.added, r.updated), (vec!["EUR_USD".to_string()], vec!["NVDA".to_string()]));
        let syms: Vec<_> = f.lists["tomorrow"].iter().map(|e| e.symbol.as_str()).collect();
        assert_eq!(syms, ["NVDA", "AMD", "EUR_USD"]);
        assert_eq!(f.lists["tomorrow"][0].why.as_deref(), Some("gap up"));
        assert_eq!(f.lists["tomorrow"][1].why.as_deref(), Some("earnings"));
        // No fields given: nothing changes, nothing "updated".
        let r = add(&mut f, "tomorrow", &["AMD".into()], &Plan::default(), now());
        assert!(r.added.is_empty() && r.updated.is_empty());
        assert_eq!(f.lists["tomorrow"][1].why.as_deref(), Some("earnings"));
        let gone = remove(&mut f, "tomorrow", &["AMD".into(), "TSLA".into()]).unwrap();
        assert_eq!(gone, ["AMD"]);
        assert!(remove(&mut f, "nope", &["X".into()]).is_err());
    }

    #[test]
    fn set_writes_one_field_and_keeps_the_rest() {
        let mut f = ListsFile::default();
        let plan = Plan {
            why: Some("earnings +39%".into()),
            constraints: Some("stop below Fri low".into()),
            ..Plan::default()
        };
        add(&mut f, "mon-11-9", &["NET".into()], &plan, now());
        let e = set(
            &mut f,
            "mon-11-9",
            "NET",
            &Plan {
                call: Some("continues".into()),
                ..Plan::default()
            },
        )
        .unwrap();
        assert_eq!(e.call.as_deref(), Some("continues"));
        assert_eq!(e.why.as_deref(), Some("earnings +39%"));
        assert_eq!(e.constraints.as_deref(), Some("stop below Fri low"));
        // An empty value clears that field only.
        let e = set(&mut f, "mon-11-9", "NET", &Plan { constraints: Some(" ".into()), ..Plan::default() }).unwrap();
        assert_eq!((e.constraints, e.call.as_deref()), (None, Some("continues")));
        // Errors are validation errors: no fields, unknown symbol, no list.
        for err in [
            set(&mut f, "mon-11-9", "NET", &Plan::default()).unwrap_err(),
            set(&mut f, "mon-11-9", "AMD", &Plan::why("x")).unwrap_err(),
            set(&mut f, "nope", "NET", &Plan::why("x")).unwrap_err(),
        ] {
            assert!(err.downcast_ref::<ValidationError>().is_some(), "{err}");
        }
    }

    #[test]
    fn old_note_only_files_read_note_as_why_and_write_why() {
        let old = r#"{"version":1,"lists":{"mon-11-9":[{"symbol":"NET","note":"earnings","added":"2026-11-09T06:09:58Z"}]}}"#;
        let f: ListsFile = serde_json::from_str(old).unwrap();
        let e = &f.lists["mon-11-9"][0];
        assert_eq!((e.why.as_deref(), e.call.as_deref()), (Some("earnings"), None));
        let out = serde_json::to_string(&f).unwrap();
        assert!(out.contains(r#""why":"earnings""#) && !out.contains("note"), "{out}");
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
                            add(f, "t", &[format!("S{i}_{j}")], &Plan::default(), now());
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
            add(f, "tomorrow", &["SPY".into()], &Plan::why("open drive"), now());
            Ok(())
        })
        .unwrap();
        let f = load_at(&path).unwrap();
        assert_eq!(f.lists["tomorrow"][0].why.as_deref(), Some("open drive"));
    }
}
