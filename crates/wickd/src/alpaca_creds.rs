//! Alpaca market-data credentials.
//!
//! Resolution order:
//! 1. `APCA_API_KEY_ID` + `APCA_API_SECRET_KEY` (Alpaca's own env var names),
//!    both set and non-empty — for agents/CI; never written anywhere.
//! 2. The OS keychain: service `wickd`, account `alpaca`, holding
//!    `{"key_id":…,"secret":…}` as one item (one ACL, one prompt), written by
//!    `wickd alpaca login`.
//!
//! The keychain read goes through the same time-bounded helper as the OANDA
//! key so a stale ACL fails with a pointer instead of hanging.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use wickd_core::equities::alpaca::AlpacaCredentials;

use crate::vault_store::{get_password_bounded, KEYRING_SERVICE};

const KEYRING_ACCOUNT: &str = "alpaca";
pub const ENV_KEY_ID: &str = "APCA_API_KEY_ID";
pub const ENV_SECRET: &str = "APCA_API_SECRET_KEY";

#[derive(Serialize, Deserialize)]
struct Stored {
    key_id: String,
    secret: String,
}

/// Where resolved credentials came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Env,
    Keychain,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Env => "env",
            Source::Keychain => "keychain",
        }
    }
}

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|e| anyhow!("could not open the keychain entry for {KEYRING_ACCOUNT}: {e}"))
}

fn from_env() -> Option<AlpacaCredentials> {
    let key_id = std::env::var(ENV_KEY_ID)
        .ok()
        .filter(|v| !v.trim().is_empty())?;
    let secret = std::env::var(ENV_SECRET)
        .ok()
        .filter(|v| !v.trim().is_empty())?;
    Some(AlpacaCredentials {
        key_id: key_id.trim().to_string(),
        secret: secret.trim().to_string(),
    })
}

/// Resolve credentials, or `None` when nothing is configured.
pub fn resolve() -> Result<Option<(AlpacaCredentials, Source)>> {
    if let Some(c) = from_env() {
        return Ok(Some((c, Source::Env)));
    }
    match get_password_bounded(entry()?, KEYRING_ACCOUNT)? {
        Ok(raw) => {
            let s: Stored = serde_json::from_str(&raw).context(
                "the stored Alpaca credentials are unreadable — run `wickd alpaca login` again",
            )?;
            Ok(Some((
                AlpacaCredentials {
                    key_id: s.key_id,
                    secret: s.secret,
                },
                Source::Keychain,
            )))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(anyhow!(
            "could not read the Alpaca keys from the keychain: {e}"
        )),
    }
}

/// Persist credentials to the keychain (replacing any previous item).
pub fn store(c: &AlpacaCredentials) -> Result<()> {
    let raw = serde_json::to_string(&Stored {
        key_id: c.key_id.clone(),
        secret: c.secret.clone(),
    })?;
    entry()?
        .set_password(&raw)
        .map_err(|e| anyhow!("could not write the Alpaca keys to the keychain: {e}"))
}

/// Remove the keychain item. Returns whether one existed.
pub fn delete() -> Result<bool> {
    match entry()?.delete_credential() {
        Ok(()) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(anyhow!(
            "could not remove the Alpaca keys from the keychain: {e}"
        )),
    }
}

/// `PKAB…WXYZ` — enough to recognise a key without revealing it.
pub fn mask(key_id: &str) -> String {
    let n = key_id.chars().count();
    if n <= 8 {
        return "*".repeat(n);
    }
    let head: String = key_id.chars().take(4).collect();
    let tail: String = key_id.chars().skip(n - 4).collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_hides_the_middle() {
        assert_eq!(mask("PKABCDEFGHWXYZ"), "PKAB…WXYZ");
        assert_eq!(mask("short"), "*****");
    }
}
