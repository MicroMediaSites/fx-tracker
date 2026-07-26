//! Whether an account nets its positions or hedges them (AGT-781).
//!
//! `close_position` closes by **instrument and side**, not by trade id. On a
//! netting account that is exactly right — one side of an instrument is one
//! position. On a *hedging* account it is not: several trades can be open on
//! the same side at once, and a side close takes out **every one of them**,
//! silently, with nothing in the response saying that is what happened.
//!
//! wickd targets netting accounts (Matt, 2026-07-25). Rather than leave that
//! as an unwritten assumption, the live close path asserts it and refuses to
//! submit when OANDA reports otherwise.
//!
//! ## Unknown means allowed, deliberately
//!
//! [`ensure_netting`] refuses only on a *positive* report of hedging. When
//! OANDA does not send the field the close proceeds, with a warning: failing
//! closed on an absent field would take out every live close the moment OANDA
//! trimmed a payload, which is a far likelier event than Matt acquiring a
//! hedging account. The guard can only protect where the broker tells us.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::error::{Error, Result};

use super::client::OandaClient;
use super::types::OandaAccount;

/// Per-process cache of `account id → hedging enabled`.
///
/// OANDA fixes hedging at account creation — it is not a setting that flips
/// under a running process — so one lookup per account per process is enough,
/// and the live close path does not pay a round trip for the guard on every
/// exit. A restart re-reads it.
fn cache() -> &'static Mutex<HashMap<String, bool>> {
    static CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Refuse an instrument+side close when `account` reports hedging enabled.
///
/// Pure, so both branches are testable without a network. `Ok(())` for a
/// netting account and for one that does not report the setting at all.
pub fn ensure_netting(account: &OandaAccount) -> Result<()> {
    check(&account.id, account.hedging_enabled)
}

/// The check itself, on the two values it actually needs. Split out so the
/// cached path does not have to fabricate an [`OandaAccount`] around a flag it
/// already knows.
fn check(account_id: &str, hedging_enabled: Option<bool>) -> Result<()> {
    match hedging_enabled {
        Some(true) => Err(Error::InvalidArgument(format!(
            "account {} has hedging enabled, and wickd closes by instrument and side — \
             that would close EVERY trade open on this side at once, not just the one \
             you mean. Close the specific trade by its id instead.",
            account_id
        ))),
        Some(false) => Ok(()),
        None => {
            // Not fatal: see the module docs. Worth saying out loud, because a
            // guard that cannot see anything should not look like one that
            // checked and approved.
            tracing::warn!(
                account = %account_id,
                "OANDA did not report hedgingEnabled; proceeding with the side close \
                 on the assumption this is a netting account"
            );
            Ok(())
        }
    }
}

/// [`ensure_netting`] against the client's own account, fetching the setting
/// once per process and reusing it afterwards.
///
/// A fetch failure is **not** fatal: the guard exists to catch a
/// misunderstanding about the account, not to add a new way for an exit to
/// fail. An unreachable account endpoint means the close proceeds as it did
/// before this guard existed.
pub async fn ensure_netting_account(client: &OandaClient) -> Result<()> {
    let account_id = client.account_id().to_string();

    if let Some(hedging) = cache().lock().ok().and_then(|c| c.get(&account_id).copied()) {
        return check(&account_id, Some(hedging));
    }

    let account = match super::endpoints::get_account(client).await {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(
                account = %account_id,
                error = %e,
                "could not read the account to check its position mode; \
                 proceeding with the side close"
            );
            return Ok(());
        }
    };

    if let (Ok(mut cache), Some(hedging)) = (cache().lock(), account.hedging_enabled) {
        cache.insert(account_id, hedging);
    }
    ensure_netting(&account)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_netting_account_closes_as_before() {
        assert!(check("101-001-1-001", Some(false)).is_ok());
    }

    // AC2: refused before anything is submitted, and the message says what
    // would have happened and what to do instead.
    #[test]
    fn a_hedging_account_is_refused_with_the_reason_and_the_alternative() {
        let err = check("101-001-1-002", Some(true)).unwrap_err();
        let msg = format!("{err}");

        assert!(msg.contains("101-001-1-002"), "names the account: {msg}");
        assert!(msg.contains("EVERY trade"), "says what it would do: {msg}");
        assert!(msg.contains("by its id"), "names the alternative: {msg}");
    }

    #[test]
    fn an_account_that_does_not_report_the_setting_is_allowed() {
        // Failing closed here would break every live close the moment OANDA
        // trimmed the payload — see the module docs.
        assert!(check("101-001-1-003", None).is_ok());
    }

    #[test]
    fn the_setting_round_trips_from_oandas_json() {
        let json = serde_json::json!({
            "id": "101-001-1-004",
            "currency": "USD",
            "balance": "10000.0000",
            "NAV": "10000.0000",
            "hedgingEnabled": true
        });

        let parsed: OandaAccount = serde_json::from_value(json).expect("decodes");

        assert_eq!(parsed.hedging_enabled, Some(true));
        assert!(ensure_netting(&parsed).is_err());
    }

    #[test]
    fn an_account_payload_without_the_field_parses_as_unknown() {
        // The pre-AGT-781 shape: absent, not false. The two must stay
        // distinguishable, or "we did not check" reads as "we checked".
        let json = serde_json::json!({
            "id": "101-001-1-005",
            "currency": "USD",
            "balance": "10000.0000",
            "NAV": "10000.0000"
        });

        let parsed: OandaAccount = serde_json::from_value(json).expect("decodes");

        assert_eq!(parsed.hedging_enabled, None);
    }
}
