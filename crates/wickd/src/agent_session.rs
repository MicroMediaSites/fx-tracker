//! The agent's saved session (`~/.wickd/agent/session.json`, wickd-learn
//! D-22, AGT-2548).
//!
//! `wickd agent ask` drives the `claude` CLI and resumes the same Claude
//! Code conversation on every ask, so the chat survives restarts. This file
//! holds the conversation's session id plus a little bookkeeping (turns,
//! compactions). `/clear` deletes it, so the next ask starts a new
//! conversation. The conversation itself lives in Claude Code's own store;
//! the agent directory doubles as the `claude` process's working directory,
//! which is what `--resume` keys sessions by, so it must stay stable.
//!
//! Writes are a tmp file + rename, so a reader never sees a half-written
//! session. The store honours `WICKD_HOME` like every other wickd store.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

const SESSION_FILE: &str = "session.json";

/// The agent directory under the wickd data home.
pub fn agent_dir() -> Result<PathBuf> {
    Ok(wickd_core::paths::wickd_data_home()
        .map_err(|e| anyhow!(e))?
        .join("agent"))
}

/// The saved conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    /// Claude Code's session id, passed back as `--resume`.
    pub session_id: String,
    /// When this conversation started (RFC 3339).
    pub created_at: String,
    /// When it was last asked or compacted (RFC 3339).
    pub updated_at: String,
    /// Questions answered in this conversation (compaction doesn't reset it).
    #[serde(default)]
    pub turns: u32,
    /// How many times `/compact` ran on it.
    #[serde(default)]
    pub compactions: u32,
    /// When `/compact` last ran (RFC 3339).
    #[serde(default)]
    pub last_compacted_at: Option<String>,
}

impl Session {
    /// A new conversation (the caller counts its turns).
    pub fn new(session_id: &str, now: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            created_at: now.to_string(),
            updated_at: now.to_string(),
            turns: 0,
            compactions: 0,
            last_compacted_at: None,
        }
    }
}

fn session_path(dir: &Path) -> PathBuf {
    dir.join(SESSION_FILE)
}

/// The saved session, or `None` when there isn't one (fresh install or
/// after `/clear`).
pub fn load_at(dir: &Path) -> Result<Option<Session>> {
    let path = session_path(dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let session: Session = serde_json::from_str(&text)
        .with_context(|| format!("{} is not a saved agent session", path.display()))?;
    Ok(Some(session))
}

/// Save the session (tmp file + rename).
pub fn save_at(dir: &Path, session: &Session) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = session_path(dir);
    let tmp = dir.join(format!(".{SESSION_FILE}.tmp"));
    let body = serde_json::to_string_pretty(session)?;
    std::fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// Forget the saved session. Returns the one that was there, if any.
pub fn clear_at(dir: &Path) -> Result<Option<Session>> {
    // An unreadable file is still cleared: `/clear` must always work.
    let previous = load_at(dir).unwrap_or(None);
    match std::fs::remove_file(session_path(dir)) {
        Ok(()) => Ok(previous),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(previous),
        Err(e) => Err(e).context("removing the saved agent session"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_session_loads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_at(dir.path()).unwrap(), None);
    }

    #[test]
    fn save_load_round_trips_and_creates_the_dir() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("agent");
        let mut s = Session::new("abc-123", "2026-11-19T10:00:00Z");
        s.turns = 3;
        save_at(&dir, &s).unwrap();
        assert_eq!(load_at(&dir).unwrap(), Some(s));
    }

    #[test]
    fn clear_returns_the_previous_session_and_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let s = Session::new("abc-123", "2026-11-19T10:00:00Z");
        save_at(dir.path(), &s).unwrap();
        assert_eq!(clear_at(dir.path()).unwrap(), Some(s));
        assert_eq!(load_at(dir.path()).unwrap(), None);
        // Clearing twice is fine.
        assert_eq!(clear_at(dir.path()).unwrap(), None);
    }

    #[test]
    fn clear_removes_a_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(SESSION_FILE), "not json").unwrap();
        assert!(load_at(dir.path()).is_err());
        assert_eq!(clear_at(dir.path()).unwrap(), None);
        assert_eq!(load_at(dir.path()).unwrap(), None);
    }
}
