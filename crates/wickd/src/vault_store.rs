//! Local credential storage for the CLI.
//!
//! OANDA **API keys** live in the OS keychain (macOS Keychain via the `keyring`
//! crate) under `service = "wickd"`. The environment-level item is keyed by the
//! environment (`"practice" | "live"`); a *named* account (AGT-625) may hold its
//! own dedicated item keyed `"<env>/<name>"` (e.g. `"practice/h004"`). The
//! keychain is gated by your login session, so there is **no master password**
//! — once you've run `wickd login`, every command reads the key straight from
//! the keychain with no prompt. That makes the CLI usable non-interactively
//! (agents, daemons, CI) without a TTY.
//!
//! The **account id** (not a secret — it's only an OANDA routing key, and the
//! desktop app stored it in plaintext too) and the active environment live in a
//! plaintext `~/.wickd/config.json` written `0600`.
//!
//! ## Named multi-account config (AGT-625)
//!
//! One OANDA practice login fans out into several practice *sub-accounts* —
//! one per strategy — so the config supports **named accounts** per
//! environment (config schema v2). Two credential shapes both work:
//!
//! - **Shared token (primary):** every sub-account id shares the ONE
//!   environment-level API key. Named accounts carry only an `account_id` in
//!   the config; key resolution falls back to the environment keychain item.
//! - **Dedicated token:** a named account may store its own keychain item
//!   (`"<env>/<name>"`), which takes precedence over the shared fallback.
//!
//! A v1 single-account config keeps working with **no migration step**: its
//! one account is addressable as the account named `"default"`, and commands
//! that don't pass `--account` resolve to it.
//!
//! Note: macOS gates keychain reads by the *binary's* code signature, so the
//! first read by a given `wickd` binary may surface a one-time GUI
//! "wickd wants to use the keychain — Always Allow" dialog. That is a click,
//! not a TTY prompt, and "Always Allow" persists it. Rebuilding an unsigned
//! (debug) binary changes the signature and can re-ask; a release-installed
//! binary is stable.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use wickd_core::config::OandaEnvironment;

/// Keychain service name. The environment-level item is keyed by the
/// environment ("practice" / "live"), a dedicated named-account item by
/// "<env>/<name>"; the stored secret is always the OANDA API key.
const KEYRING_SERVICE: &str = "wickd";

/// The implicit account name a v1 single-account config answers to, and the
/// account every command resolves when `--account` isn't passed (AGT-625 AC3).
pub const DEFAULT_ACCOUNT: &str = "default";

/// Plaintext, non-secret config: per-environment account ids + the active env.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// Format version. 1 = single account per env; 2 = named accounts too.
    pub version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub practice: Option<EnvConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live: Option<EnvConfig>,
    /// Which environment the user last logged into ("practice" | "live").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnvConfig {
    /// Plaintext OANDA account id (not a secret) of the `default` account —
    /// the v1 single-account slot, kept verbatim so an unmigrated v1 config
    /// deserializes and resolves unchanged (AGT-625 AC3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// Named accounts (config v2, AGT-625 AC1), e.g. `h004`, `h015` — one
    /// OANDA practice sub-account per strategy. BTreeMap for stable JSON and
    /// stable `--status` / error-message ordering.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, AccountConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountConfig {
    /// Plaintext OANDA account id (not a secret).
    pub account_id: String,
}

impl EnvConfig {
    /// Resolve the OANDA account id for a named account (pure — no keychain).
    /// `default` answers from the v1 slot OR from an explicit `accounts` entry;
    /// any other name must be in `accounts`. Unknown names get an error that
    /// lists what IS configured.
    pub fn account_id_for(&self, env: OandaEnvironment, account: &str) -> Result<String> {
        if let Some(a) = self.accounts.get(account) {
            return Ok(a.account_id.clone());
        }
        if account == DEFAULT_ACCOUNT {
            if let Some(id) = &self.account_id {
                return Ok(id.clone());
            }
        }
        // Both messages carry the "credentials" token so the per-command exit
        // classifiers (trade / watch / stream, which match on it) route an
        // unresolvable account to exit::AUTH, same as any missing-login error.
        let known = self.account_names();
        if known.is_empty() {
            bail!(
                "no {env} credentials stored — run `wickd login --env {env}`",
                env = env_str(env)
            );
        }
        bail!(
            "no credentials for {env} account '{account}' (configured: {names}) — run `wickd login --env {env} --account {account}`",
            env = env_str(env),
            names = known.join(", "),
        );
    }

    /// Every addressable account name, `default` first (if present), then the
    /// named accounts in stable (sorted) order. No secrets involved.
    pub fn account_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        if self.account_id.is_some() {
            names.push(DEFAULT_ACCOUNT.to_string());
        }
        for name in self.accounts.keys() {
            if name != DEFAULT_ACCOUNT || self.account_id.is_none() {
                names.push(name.clone());
            }
        }
        names
    }
}

/// Validate a `--account` name before it becomes a config key / keychain key.
/// Kept strict (lowercase alphanumeric + `-`/`_`) so names are shell-safe and
/// can never smuggle a path separator into the keychain key.
pub fn validate_account_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        bail!("account name must be 1-64 characters");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        bail!(
            "invalid account name '{name}' — use lowercase letters, digits, '-' or '_' (e.g. h004)"
        );
    }
    Ok(())
}

/// Canonical short name for an environment.
pub fn env_str(env: OandaEnvironment) -> &'static str {
    match env {
        OandaEnvironment::Practice => "practice",
        OandaEnvironment::Live => "live",
    }
}

/// Path to the non-secret config file (`~/.wickd/config.json`).
pub fn config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not resolve home directory")?;
    Ok(home.join(".wickd").join("config.json"))
}

/// Path to the legacy master-password vault (`~/.wickd/creds.enc`). Only
/// referenced so `logout` can clean it up after the keychain migration.
fn legacy_vault_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not resolve home directory")?;
    Ok(home.join(".wickd").join("creds.enc"))
}

/// Load the config file, or an empty one if it doesn't exist yet.
pub fn load() -> Result<Config> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(Config { version: 1, ..Default::default() });
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("reading config at {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| "config file is corrupt or not valid JSON")
}

/// Write the config file with `0600` permissions (owner-only).
pub fn save(config: &Config) -> Result<PathBuf> {
    let path = config_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let body = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, body).with_context(|| format!("writing config at {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(path)
}

/// Keychain item key for `account` in `env` (pure). The `default` account owns
/// the environment-level item (`"practice"` — the v1 layout, unchanged, so
/// stored v1 credentials stay reachable); a named account's dedicated item is
/// `"<env>/<name>"`. `/` cannot appear in a validated account name, so the two
/// namespaces can't collide.
pub fn keyring_account_key(env: OandaEnvironment, account: &str) -> String {
    if account == DEFAULT_ACCOUNT {
        env_str(env).to_string()
    } else {
        format!("{}/{}", env_str(env), account)
    }
}

/// Keychain entry holding `account`'s API key in `env`.
fn keyring_entry(env: OandaEnvironment, account: &str) -> Result<keyring::Entry> {
    let key = keyring_account_key(env, account);
    keyring::Entry::new(KEYRING_SERVICE, &key)
        .map_err(|e| anyhow!("could not open the keychain entry for {key}: {e}"))
}

/// Read the environment-level (shared) API key, if one is stored. Used by
/// `login --account <name>` to reuse the shared token instead of demanding the
/// key be pasted again for every sub-account.
pub fn shared_api_key(env: OandaEnvironment) -> Result<Option<String>> {
    match keyring_entry(env, DEFAULT_ACCOUNT)?.get_password() {
        Ok(k) => Ok(Some(k)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(other) => Err(anyhow!("could not read the API key from the keychain: {other}")),
    }
}

fn env_config_mut(cfg: &mut Config, env: OandaEnvironment) -> &mut EnvConfig {
    let slot = match env {
        OandaEnvironment::Practice => &mut cfg.practice,
        OandaEnvironment::Live => &mut cfg.live,
    };
    slot.get_or_insert_with(EnvConfig::default)
}

/// The config format version the current contents require: v2 as soon as any
/// named account exists, else v1 (a pure-default config keeps its v1 shape).
fn config_version(cfg: &Config) -> u32 {
    let has_named = [&cfg.practice, &cfg.live]
        .into_iter()
        .flatten()
        .any(|e| !e.accounts.is_empty());
    if has_named {
        2
    } else {
        1
    }
}

/// Store credentials for `account` in `env` and set `env` active. The account
/// id goes into the config; `api_key` handling encodes the two AGT-625 shapes:
///
/// - `Some(key)` — write a keychain item for this account (the environment
///   item for `default`, a dedicated `"<env>/<name>"` item otherwise).
/// - `None` — config-only registration (shared-token shape): the named
///   account will resolve the environment-level key via the fallback. Only
///   meaningful for named accounts; `default` always passes `Some`.
pub fn store(
    env: OandaEnvironment,
    account: &str,
    api_key: Option<&str>,
    account_id: &str,
) -> Result<()> {
    validate_account_name(account)?;
    if let Some(key) = api_key {
        keyring_entry(env, account)?
            .set_password(key)
            .map_err(|e| anyhow!("could not write the API key to the keychain: {e}"))?;
    }

    let mut cfg = load()?;
    let env_cfg = env_config_mut(&mut cfg, env);
    if account == DEFAULT_ACCOUNT {
        // The v1 slot IS the default account — keep writing it there so a
        // pre-multi-account config stays byte-compatible (AC3).
        env_cfg.account_id = Some(account_id.to_string());
    } else {
        env_cfg
            .accounts
            .insert(account.to_string(), AccountConfig { account_id: account_id.to_string() });
    }
    cfg.version = config_version(&cfg);
    cfg.active = Some(env_str(env).to_string());
    save(&cfg)?;
    Ok(())
}

/// Resolve `(api_key, account_id)` for `account` in `env` — account id from
/// the config, API key from the keychain: the account's dedicated item if it
/// has one, else the shared environment-level item (AGT-625 AC1, the
/// primary shape: many sub-account ids under one OANDA login token). No
/// password; reads are non-interactive.
pub fn credentials(env: OandaEnvironment, account: &str) -> Result<(String, String)> {
    let cfg = load()?;
    let env_cfg = match env {
        OandaEnvironment::Practice => cfg.practice,
        OandaEnvironment::Live => cfg.live,
    }
    .ok_or_else(|| {
        anyhow!(
            "no {} credentials stored — run `wickd login --env {}`",
            env_str(env),
            env_str(env)
        )
    })?;

    let account_id = env_cfg.account_id_for(env, account)?;

    // Dedicated item first; a named account without one falls back to the
    // shared environment-level token.
    let api_key = match keyring_entry(env, account)?.get_password() {
        Ok(k) => k,
        Err(keyring::Error::NoEntry) if account != DEFAULT_ACCOUNT => {
            shared_api_key(env)?.ok_or_else(|| {
                anyhow!(
                    "no API key in the keychain for {} account '{}' (and no shared {} key to fall back to) — run `wickd login --env {}`",
                    env_str(env),
                    account,
                    env_str(env),
                    env_str(env)
                )
            })?
        }
        Err(keyring::Error::NoEntry) => bail!(
            "no {} API key in the keychain — run `wickd login --env {}`",
            env_str(env),
            env_str(env)
        ),
        Err(other) => bail!("could not read the API key from the keychain: {other}"),
    };

    Ok((api_key, account_id))
}

/// Remove credentials for one environment (or all if `env` is `None`): deletes
/// the keychain item(s) and drops the account id(s) from the config. Returns
/// true if anything was removed.
pub fn remove(env: Option<OandaEnvironment>) -> Result<bool> {
    let targets = match env {
        Some(e) => vec![e],
        None => vec![OandaEnvironment::Practice, OandaEnvironment::Live],
    };

    let mut removed = false;
    let mut cfg = load()?;
    for e in targets {
        let taken = match e {
            OandaEnvironment::Practice => cfg.practice.take(),
            OandaEnvironment::Live => cfg.live.take(),
        };
        // Delete the environment-level item plus every named account's
        // dedicated item; "no entry" just means it was already gone (a
        // shared-token named account never had one).
        let mut keys = vec![DEFAULT_ACCOUNT.to_string()];
        if let Some(env_cfg) = &taken {
            keys.extend(env_cfg.accounts.keys().cloned());
        }
        for key in keys {
            match keyring_entry(e, &key)?.delete_credential() {
                Ok(()) => removed = true,
                Err(keyring::Error::NoEntry) => {}
                Err(other) => return Err(anyhow!("could not delete the keychain entry: {other}")),
            }
        }
        removed |= taken.is_some();
        if cfg.active.as_deref() == Some(env_str(e)) {
            cfg.active = None;
        }
    }
    cfg.version = config_version(&cfg);

    if env.is_none() {
        // Full logout: remove the config file and any legacy encrypted vault.
        let cpath = config_path()?;
        if cpath.exists() {
            std::fs::remove_file(&cpath)?;
            removed = true;
        }
        if let Ok(legacy) = legacy_vault_path() {
            let _ = std::fs::remove_file(legacy); // best-effort migration cleanup
        }
    } else {
        save(&cfg)?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_str_is_stable() {
        // These strings are the keychain item account names + config keys; they
        // must not drift or stored credentials become unreachable.
        assert_eq!(env_str(OandaEnvironment::Practice), "practice");
        assert_eq!(env_str(OandaEnvironment::Live), "live");
    }

    fn named(env_cfg: &mut EnvConfig, name: &str, id: &str) {
        env_cfg.accounts.insert(name.to_string(), AccountConfig { account_id: id.into() });
    }

    #[test]
    fn config_round_trips_and_holds_no_secret() {
        let mut practice = EnvConfig { account_id: Some("101-001-00000000-005".into()), ..Default::default() };
        named(&mut practice, "h004", "101-001-00000000-001");
        let cfg = Config {
            version: 2,
            practice: Some(practice),
            live: None,
            active: Some("practice".into()),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        // The on-disk config carries only the non-secret account ids + active
        // env; the API key never appears here (it lives in the keychain).
        assert!(json.contains("101-001-00000000-005"));
        assert!(json.contains("h004"));
        assert!(!json.to_lowercase().contains("api_key"));
        assert!(!json.to_lowercase().contains("blob"));
        assert!(!json.to_lowercase().contains("password"));

        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.active.as_deref(), Some("practice"));
        let practice = back.practice.unwrap();
        assert_eq!(practice.account_id.as_deref(), Some("101-001-00000000-005"));
        assert_eq!(practice.accounts["h004"].account_id, "101-001-00000000-001");
        assert!(back.live.is_none());
    }

    #[test]
    fn empty_config_default_is_versioned() {
        let cfg: Config = serde_json::from_str("{\"version\":1}").unwrap();
        assert_eq!(cfg.version, 1);
        assert!(cfg.practice.is_none() && cfg.live.is_none() && cfg.active.is_none());
    }

    /// AC3: an existing v1 single-account config — the exact JSON `store`
    /// wrote before AGT-625 — deserializes with no migration step, and its one
    /// account answers to the name `default`.
    #[test]
    fn v1_config_is_the_default_account_with_no_migration() {
        let raw = r#"{"version":1,"practice":{"account_id":"101-001-00000000-005"},"active":"practice"}"#;
        let cfg: Config = serde_json::from_str(raw).unwrap();
        let practice = cfg.practice.unwrap();
        assert!(practice.accounts.is_empty());
        assert_eq!(
            practice.account_id_for(OandaEnvironment::Practice, DEFAULT_ACCOUNT).unwrap(),
            "101-001-00000000-005"
        );
        // ...and it re-serializes without inventing v2 fields.
        let json = serde_json::to_string(&practice).unwrap();
        assert!(!json.contains("accounts"));
    }

    /// AC1/AC2: named accounts resolve their own account id; an unknown name
    /// fails with the configured names listed (not a silent default).
    #[test]
    fn named_accounts_resolve_and_unknown_names_are_listed_errors() {
        let mut env_cfg = EnvConfig { account_id: Some("101-001-00000000-005".into()), ..Default::default() };
        named(&mut env_cfg, "h004", "101-001-00000000-001");
        named(&mut env_cfg, "h015", "101-001-00000000-002");

        let env = OandaEnvironment::Practice;
        assert_eq!(env_cfg.account_id_for(env, "h004").unwrap(), "101-001-00000000-001");
        assert_eq!(env_cfg.account_id_for(env, "h015").unwrap(), "101-001-00000000-002");
        // AC3: no --account → the v1 slot, addressed as `default`.
        assert_eq!(env_cfg.account_id_for(env, DEFAULT_ACCOUNT).unwrap(), "101-001-00000000-005");

        let err = env_cfg.account_id_for(env, "h999").unwrap_err().to_string();
        assert!(err.contains("no credentials for practice account 'h999'"), "{err}");
        assert!(err.contains("default, h004, h015"), "{err}");
        assert!(err.contains("--account h999"), "{err}");
        // The "credentials" token routes this to exit::AUTH in every
        // per-command classifier (trade/watch/stream) — keep it.
        assert!(err.contains("credentials"), "{err}");

        // A wholly-unconfigured env points at plain login, not a name list.
        let empty = EnvConfig::default();
        let err = empty.account_id_for(env, DEFAULT_ACCOUNT).unwrap_err().to_string();
        assert!(err.contains("no practice credentials stored"), "{err}");
    }

    /// AC4: `login --status` sources its names here — `default` first, then
    /// named accounts in stable order, deduped if someone hand-writes an
    /// explicit `default` entry into `accounts`.
    #[test]
    fn account_names_are_stable_and_deduped() {
        let mut env_cfg = EnvConfig { account_id: Some("id-default".into()), ..Default::default() };
        named(&mut env_cfg, "h015", "id-15");
        named(&mut env_cfg, "h004", "id-4");
        assert_eq!(env_cfg.account_names(), vec!["default", "h004", "h015"]);

        // Hand-written accounts.default: listed once, and it WINS resolution
        // (the accounts map is checked first).
        named(&mut env_cfg, "default", "id-map");
        assert_eq!(env_cfg.account_names(), vec!["default", "h004", "h015"]);
        assert_eq!(
            env_cfg.account_id_for(OandaEnvironment::Practice, DEFAULT_ACCOUNT).unwrap(),
            "id-map"
        );

        assert!(EnvConfig::default().account_names().is_empty());
    }

    /// The keychain layout: `default` keeps the v1 environment-level item name
    /// (stored credentials stay reachable); named accounts get "<env>/<name>".
    #[test]
    fn keyring_keys_preserve_v1_layout_and_namespace_named_accounts() {
        assert_eq!(keyring_account_key(OandaEnvironment::Practice, DEFAULT_ACCOUNT), "practice");
        assert_eq!(keyring_account_key(OandaEnvironment::Live, DEFAULT_ACCOUNT), "live");
        assert_eq!(keyring_account_key(OandaEnvironment::Practice, "h004"), "practice/h004");
        assert_eq!(keyring_account_key(OandaEnvironment::Live, "h004"), "live/h004");
    }

    #[test]
    fn account_names_are_validated() {
        for ok in ["h004", "h015", "default", "a", "my_acct-2"] {
            assert!(validate_account_name(ok).is_ok(), "{ok} should be valid");
        }
        for bad in ["", "H004", "has space", "a/b", "a:b", "émoji", &"x".repeat(65)] {
            assert!(validate_account_name(bad).is_err(), "{bad:?} should be invalid");
        }
    }

    /// The config self-reports v2 only once a named account exists; a
    /// pure-default config keeps its v1 shape byte-for-byte.
    #[test]
    fn version_stays_v1_until_a_named_account_appears() {
        let mut cfg = Config {
            version: 1,
            practice: Some(EnvConfig { account_id: Some("id".into()), ..Default::default() }),
            live: None,
            active: Some("practice".into()),
        };
        assert_eq!(config_version(&cfg), 1);
        named(cfg.practice.as_mut().unwrap(), "h004", "id-4");
        assert_eq!(config_version(&cfg), 2);
    }
}
