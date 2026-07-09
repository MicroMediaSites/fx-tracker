//! Local store access for the wickd MCP server (AGT-649).
//!
//! The server reads/writes the same SQLite store the desktop app owns:
//! **`~/.wickd/app.db`**. Schema is owned by the app
//! (`src-tauri/src/local_store/migrations.rs`) — this module includes that
//! exact file via `#[path]` so there is a single migration source of truth
//! and the server can bring a store up to date idempotently on open
//! (same behavior as the app; `PRAGMA user_version` gates each step).
//!
//! `WICKD_DB_PATH` overrides the store location — used by tests and smoke
//! runs so they never touch the live store.

use std::path::PathBuf;

use rusqlite::Connection;

/// The app's migration list, included from the single source of truth in
/// `src-tauri`. Only depends on `rusqlite`, so the include stays light.
#[path = "../../src-tauri/src/local_store/migrations.rs"]
pub mod migrations;

/// Resolve the store path: `WICKD_DB_PATH` env override, else `~/.wickd/app.db`.
pub fn resolve_db_path() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("WICKD_DB_PATH") {
        if !p.trim().is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    let home = dirs::home_dir().ok_or_else(|| "could not resolve home directory".to_string())?;
    Ok(home.join(".wickd").join("app.db"))
}

/// Open the store at `path`, creating parent dirs if needed, and apply any
/// unapplied migrations (idempotent). WAL journal mode, matching the app.
pub fn open(path: &std::path::Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("could not create {}: {}", parent.display(), e))?;
    }
    let mut conn = Connection::open(path)
        .map_err(|e| format!("could not open {}: {}", path.display(), e))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("could not set WAL mode: {}", e))?;
    migrations::apply(&mut conn).map_err(|e| format!("migration failed: {}", e))?;
    Ok(conn)
}
