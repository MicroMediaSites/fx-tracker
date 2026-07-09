//! Owner-only permissions for local wickd data files (AGT-668).
//!
//! The local SQLite stores (`audit.db`, `baselines.db`, `spreads.db`) live under
//! `~/.wickd/` and hold trading data that no other local user should be able to
//! read. `rusqlite::Connection::open` creates the file honouring the process
//! umask, which on a default umask leaves it world-readable — so after opening we
//! tighten the file to `0600` (owner read/write only). The call is idempotent, so
//! re-opening an existing store simply re-asserts the mode (additive; never
//! loosens).

use std::path::Path;

use anyhow::{Context, Result};

/// Restrict a file to owner read/write only (`0600`). No-op on non-Unix.
#[cfg(unix)]
pub fn restrict_file(path: impl AsRef<Path>) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let path = path.as_ref();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("could not set 0600 on {}", path.display()))
}

/// Restrict a file to owner read/write only (`0600`). No-op on non-Unix.
#[cfg(not(unix))]
pub fn restrict_file(_path: impl AsRef<Path>) -> Result<()> {
    Ok(())
}
