//! `wickd login` — store OANDA credentials (API key in the OS keychain).
//!
//!   wickd login [--env practice|live] [--account-id A]
//!   wickd login --env practice --account h004 --account-id A   # named account (AGT-625)
//!   wickd login --status
//!   wickd logout [--env practice|live]
//!
//! The API key is never accepted as a CLI flag (AGT-663: flags leak into
//! process args and shell history). It is read from stdin — interactively with
//! no echo on a TTY, or piped for agents/CI (`printf '%s' "$KEY" | wickd login
//! --account-id A`).
//!
//! `--account <name>` stores credentials for a *named* account within the
//! environment (one OANDA practice sub-account per strategy). If the
//! environment already holds a shared token in the keychain, the named account
//! reuses it (config-only registration — the primary shape: many sub-account
//! ids under one OANDA login token). Otherwise the key read from stdin is
//! stored as a dedicated keychain item for that account.
//!
//! Credentials are validated against OANDA (a real account fetch) before being
//! stored, so a successful `login` means the keys actually work. The API key
//! goes into the OS keychain and the (non-secret) account id into
//! `~/.wickd/config.json` — there is no master password, so every other command
//! reads them without a prompt. See `vault_store` for details.

use anyhow::{anyhow, Context, Result};
use clap::Args;

use wickd_core::config::OandaEnvironment;
use wickd_core::oanda::endpoints;

use crate::output::{exit, Out};
use crate::prompt;
use crate::vault_store::{self, env_str};

#[derive(Args, Debug)]
pub struct LoginArgs {
    /// Which OANDA environment these credentials are for.
    #[arg(long, default_value = "practice")]
    pub env: String,
    /// Named account within --env to store credentials for (AGT-625), e.g.
    /// h004. Default: the single/default account.
    #[arg(long, default_value = vault_store::DEFAULT_ACCOUNT)]
    pub account: String,
    /// OANDA account id (omit to be prompted).
    #[arg(long)]
    pub account_id: Option<String>,
    /// Report which environments are configured, then exit. No secrets shown.
    #[arg(long)]
    pub status: bool,
}

#[derive(Args, Debug)]
pub struct LogoutArgs {
    /// Only remove this environment's credentials (default: remove all).
    #[arg(long)]
    pub env: Option<String>,
}

pub async fn run(args: LoginArgs, out: Out) -> ! {
    if args.status {
        status(out);
    }
    match login(&args, out).await {
        Ok(()) => std::process::exit(exit::OK),
        Err(e) => out.fail(exit::GENERIC, "login_failed", format!("{e:#}")),
    }
}

async fn login(args: &LoginArgs, out: Out) -> Result<()> {
    let env = OandaEnvironment::from_str(&args.env)
        .map_err(|e| anyhow!(e.to_string()))
        .with_context(|| "invalid --env")?;
    vault_store::validate_account_name(&args.account).with_context(|| "invalid --account")?;

    // Gather the API key. A NAMED account reuses the shared environment-level
    // token already in the keychain (AGT-625's primary shape: many practice
    // sub-account ids under one OANDA login token) — registration is then
    // config-only and no new keychain item is written. Otherwise the key is
    // read from stdin: interactive no-echo on a TTY, or piped for agents/CI.
    // The key is never taken from a CLI flag (AGT-663: keeps it out of process
    // args and shell history).
    let shared = if args.account != vault_store::DEFAULT_ACCOUNT {
        vault_store::shared_api_key(env)?
    } else {
        None
    };
    let reused_shared = shared.is_some();
    let api_key = match shared {
        Some(k) => k,
        None => prompt::secret(&format!("OANDA {} API key: ", env_str(env)))?,
    };
    let account_id = match &args.account_id {
        Some(a) => a.clone(),
        None => prompt::line(&format!(
            "OANDA {} account id for '{}': ",
            env_str(env),
            args.account
        ))?,
    };
    if api_key.trim().is_empty() || account_id.trim().is_empty() {
        out.fail(exit::VALIDATION, "missing_credentials", "API key and account id are required");
    }

    // Validate against OANDA before persisting (real account fetch).
    let client = wickd_core::oanda::OandaClient::with_credentials(&api_key, &account_id, env)
        .map_err(|e| anyhow!("could not build OANDA client: {e}"))?;
    let account = match endpoints::get_account(&client).await {
        Ok(a) => a,
        Err(_) => out.fail(
            exit::OANDA,
            "credentials_rejected",
            format!(
                "OANDA rejected these {} credentials (check API key, account id, and environment)",
                env_str(env)
            ),
        ),
    };

    // API key → OS keychain (skipped when the shared token is reused);
    // account id → ~/.wickd/config.json. No password.
    vault_store::store(
        env,
        &args.account,
        (!reused_shared).then_some(api_key.as_str()),
        &account_id,
    )?;

    out.ok(&serde_json::json!({
        "ok": true,
        "environment": env_str(env),
        "account": args.account,
        "account_id": account.id,
        "currency": account.currency,
        "balance": account.balance,
        "stored": "keychain",
        // Which keychain item this account's key resolves through: its own
        // dedicated item, or the shared environment-level token (AGT-625).
        "key_source": if reused_shared { "shared" } else { "dedicated" },
    }));
    Ok(())
}

/// Per-environment `--status` payload: every addressable account name with
/// its (non-secret) OANDA account id. The API key never appears (AGT-625 AC4).
fn env_status(env: OandaEnvironment, c: &vault_store::EnvConfig) -> serde_json::Value {
    let mut accounts = serde_json::Map::new();
    for name in c.account_names() {
        let id = c.account_id_for(env, &name).ok();
        accounts.insert(name, serde_json::json!({ "account_id": id }));
    }
    serde_json::json!({ "accounts": accounts })
}

fn status(out: Out) -> ! {
    let result = (|| -> Result<serde_json::Value> {
        let path = vault_store::config_path()?;
        let cfg = vault_store::load()?;
        Ok(serde_json::json!({
            "config": path.display().to_string(),
            "store": "keychain",
            "active": cfg.active,
            "practice": cfg.practice.as_ref().map(|c| env_status(OandaEnvironment::Practice, c)),
            "live": cfg.live.as_ref().map(|c| env_status(OandaEnvironment::Live, c)),
        }))
    })();
    match result {
        Ok(v) => {
            out.ok(&v);
            std::process::exit(exit::OK);
        }
        Err(e) => out.fail(exit::GENERIC, "status_failed", format!("{e:#}")),
    }
}

pub fn logout(args: LogoutArgs, out: Out) -> ! {
    let result = (|| -> Result<serde_json::Value> {
        let env = match &args.env {
            Some(s) => Some(OandaEnvironment::from_str(s).map_err(|e| anyhow!(e.to_string()))?),
            None => None,
        };
        let removed = vault_store::remove(env)?;
        Ok(serde_json::json!({
            "ok": true,
            "removed": removed,
            "scope": env.map(env_str).unwrap_or("all"),
        }))
    })();
    match result {
        Ok(v) => {
            out.ok(&v);
            std::process::exit(exit::OK);
        }
        Err(e) => out.fail(exit::GENERIC, "logout_failed", format!("{e:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault_store::{AccountConfig, EnvConfig};

    /// AGT-625 AC4: `login --status` lists environments and account names —
    /// account ids are fine (they're routing keys, not secrets), but no API
    /// key material can appear in the payload.
    #[test]
    fn status_lists_account_names_without_secrets() {
        let mut practice = EnvConfig {
            account_id: Some("000-000-00000000-005".into()),
            ..Default::default()
        };
        practice
            .accounts
            .insert("h004".into(), AccountConfig { account_id: "000-000-00000000-001".into() });
        practice
            .accounts
            .insert("h015".into(), AccountConfig { account_id: "000-000-00000000-002".into() });

        let v = env_status(OandaEnvironment::Practice, &practice);
        let accounts = v["accounts"].as_object().unwrap();
        assert_eq!(
            accounts.keys().collect::<Vec<_>>(),
            vec!["default", "h004", "h015"],
            "every configured account name is listed, default first"
        );
        assert_eq!(accounts["default"]["account_id"], "000-000-00000000-005");
        assert_eq!(accounts["h004"]["account_id"], "000-000-00000000-001");
        assert_eq!(accounts["h015"]["account_id"], "000-000-00000000-002");

        // No secret-shaped fields anywhere in the payload.
        let json = v.to_string().to_lowercase();
        assert!(!json.contains("api_key"));
        assert!(!json.contains("password"));
        assert!(!json.contains("token"));
    }

    /// AC3: a v1 single-account env renders as exactly one `default` account.
    #[test]
    fn status_renders_v1_config_as_the_default_account() {
        let practice: EnvConfig =
            serde_json::from_str(r#"{"account_id":"000-000-00000000-005"}"#).unwrap();
        let v = env_status(OandaEnvironment::Practice, &practice);
        let accounts = v["accounts"].as_object().unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts["default"]["account_id"], "000-000-00000000-005");
    }
}
