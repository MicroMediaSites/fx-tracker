//! Shared helper: resolve OANDA credentials → an authenticated client.
//!
//! Every data/execution command needs a client. Credentials come from `login`:
//! the API key from the OS keychain, the account id from `~/.wickd/config.json`.
//! There is no master password, so resolution is non-interactive — agents,
//! daemons, and CI work without a TTY.
//!
//! `account` selects a *named* account within the environment (AGT-625): the
//! `--account` flag on `trade` / `watch` / `approve` / `stream`. Commands that
//! don't expose the flag pass [`vault_store::DEFAULT_ACCOUNT`], which is also
//! the flag's default — so a v1 single-account setup resolves exactly as
//! before.

use anyhow::{anyhow, Result};

use wickd_core::config::OandaEnvironment;
use wickd_core::oanda::OandaClient;

use crate::vault_store;

/// Resolve `(environment, api_key, account_id)` for the given `--env` string
/// and `--account` name. Used directly by `stream` (the price streamer needs
/// the raw API key).
pub fn resolve_credentials(
    env_str: &str,
    account: &str,
) -> Result<(OandaEnvironment, String, String)> {
    let env = OandaEnvironment::from_str(env_str).map_err(|e| anyhow!(e.to_string()))?;
    let (api_key, account_id) = vault_store::credentials(env, account)?;
    Ok((env, api_key, account_id))
}

/// Resolve `(environment, client)` for the given `--env` string and
/// `--account` name.
pub fn resolve(env_str: &str, account: &str) -> Result<(OandaEnvironment, OandaClient)> {
    let (env, api_key, account_id) = resolve_credentials(env_str, account)?;
    let client = OandaClient::with_credentials(&api_key, &account_id, env)
        .map_err(|e| anyhow!("failed to construct OANDA client: {e}"))?;
    Ok((env, client))
}
