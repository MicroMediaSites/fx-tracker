//! OANDA environment and configuration commands.
//!
//! Handles switching between practice/live environments and credential management.

use serde::Serialize;
use tauri::State;
use tracing::info;

use crate::AppState;
use crate::commands::local_store::{local_get_credential, local_save_credential, LocalStoreState};
use candlesight_lib::{
    config::OandaEnvironment,
    crypto,
    local_store::LocalCredential,
    oanda::{OandaClient, endpoints},
};

/// OANDA credentials for frontend display (masked)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OandaCredentials {
    pub api_key_preview: String,
    pub account_id: String,
    pub account_alias: Option<String>,
    pub environment: String,
    pub is_configured: bool,
}

/// Switch OANDA environment between practice and live
#[tauri::command]
pub async fn switch_oanda_environment(
    environment: String,
    account_id: String,
    use_practice_url: Option<bool>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let requested_env = OandaEnvironment::from_str(&environment)
        .map_err(|e| e.to_string())?;

    let actual_env = if use_practice_url.unwrap_or(false) {
        info!("[Dev] Using practice URL for {:?} environment", requested_env);
        OandaEnvironment::Practice
    } else {
        requested_env
    };

    let (current_env, current_account_id) = {
        let client = state.client.read().await;
        (client.environment(), client.account_id().to_string())
    };

    if current_env == actual_env && current_account_id == account_id {
        info!("Already on {:?} environment with same account", actual_env);
        return Ok(format!("Already using {:?} environment", requested_env));
    }

    let mask_account = |id: &str| -> String {
        if id.len() > 4 { format!("***{}", &id[id.len()-4..]) } else { "****".to_string() }
    };
    info!("[switch_oanda_environment] Switching from {} to {} (URL: {:?} -> {:?})",
        mask_account(&current_account_id), mask_account(&account_id), current_env, actual_env);

    // AGT-652: no in-process watchers to stop - the watcher engine is the
    // wickd watch daemon, which owns its own credentials/environment.

    let api_key = {
        let credential_vault = state.credential_vault.lock().await;
        let vault = credential_vault.as_ref()
            .ok_or("Vault not unlocked - cannot switch environment")?;

        info!("[switch_oanda_environment] Vault state - has_practice: {}, has_live: {}",
            vault.has_practice(),
            vault.has_live()
        );

        match requested_env {
            OandaEnvironment::Live => {
                vault.get_live_api_key()
                    .ok_or("No live API key in vault - add live credentials first")?
                    .to_string()
            }
            OandaEnvironment::Practice => {
                vault.get_practice_api_key()
                    .ok_or("No practice API key in vault")?
                    .to_string()
            }
        }
    };

    let new_client = OandaClient::with_credentials(&api_key, &account_id, actual_env)
        .map_err(|e| format!("Failed to create client for {:?}: {}", actual_env, e))?;

    {
        let mut client = state.client.write().await;
        *client = new_client;
    }

    info!("Switched OANDA environment from {:?} to {:?} (URL: {:?})", current_env, requested_env, actual_env);

    Ok(format!("Switched to {:?} environment", requested_env))
}

/// Get the current OANDA environment
#[tauri::command]
pub async fn get_oanda_environment(
    state: State<'_, AppState>,
) -> Result<String, String> {
    let client = state.client.read().await;
    let env = client.environment();
    Ok(format!("{:?}", env).to_lowercase())
}

/// Get current OANDA credentials (masked for security)
#[tauri::command]
pub async fn get_oanda_credentials(
    state: State<'_, AppState>,
) -> Result<OandaCredentials, String> {
    let client = state.client.read().await;
    let env = client.environment();
    let has_creds = client.has_credentials();

    let account_id = if has_creds {
        client.account_id().to_string()
    } else {
        "Not configured".to_string()
    };

    let api_key_preview = if has_creds {
        "Configured (hidden)".to_string()
    } else {
        "Not configured".to_string()
    };

    let account_alias = if has_creds {
        match endpoints::get_account(&client).await {
            Ok(account) => {
                println!("[get_oanda_credentials] Account fetched, alias: {:?}", account.alias);
                account.alias
            }
            Err(e) => {
                println!("[get_oanda_credentials] Failed to fetch account: {:?}", e);
                None
            }
        }
    } else {
        println!("[get_oanda_credentials] No credentials configured");
        None
    };

    Ok(OandaCredentials {
        api_key_preview,
        account_id,
        account_alias,
        environment: format!("{:?}", env).to_lowercase(),
        is_configured: has_creds,
    })
}

/// Persist OANDA credentials through the encrypted credential vault (AGT-663).
///
/// The API key is encrypted with the master password via `crypto::vault` and the
/// resulting ciphertext blob plus the (non-secret) account id are written to the
/// local store's `credential` table — the same encrypted path onboarding and
/// settings use (`encrypt_credentials` + `local_save_credential` + `unlock_vault`).
/// No plaintext `.env` is ever written.
///
/// The previous implementation wrote `OANDA_API_KEY`/`OANDA_ACCOUNT_ID` in
/// plaintext to `~/Library/Application Support/com.openthink.wickd/.env`. That
/// location was never read back by any code path (config loading reads only the
/// legacy `com.candlesight.app` / `com.fx-tracker.app` dev `.env` files), so no
/// runtime migration is required — new saves land in the encrypted store and the
/// stale plaintext file, if present, is simply ignored (and never touched here).
///
/// The other environment's stored blob/account are preserved on write so an
/// existing vault (e.g. a configured practice slot) keeps working when a live
/// slot is added, and vice-versa.
#[tauri::command]
pub async fn save_oanda_credentials(
    master_password: String,
    api_key: String,
    account_id: String,
    environment: String,
    state: State<'_, AppState>,
    local: State<'_, LocalStoreState>,
) -> Result<String, String> {
    if api_key.trim().is_empty() {
        return Err("API key cannot be empty".to_string());
    }
    if account_id.trim().is_empty() {
        return Err("Account ID cannot be empty".to_string());
    }
    if master_password.is_empty() {
        return Err("Master password is required to encrypt credentials".to_string());
    }

    let env = match environment.to_lowercase().as_str() {
        "practice" | "demo" => OandaEnvironment::Practice,
        "live" => OandaEnvironment::Live,
        _ => {
            return Err(format!(
                "Invalid environment: {}. Use 'practice' or 'live'",
                environment
            ))
        }
    };

    let api_key = api_key.trim();
    let account_id = account_id.trim();

    // Encrypt the API key with the master password (env-scoped subkey). The
    // account id is not a secret and is stored alongside the ciphertext.
    let blob = match env {
        OandaEnvironment::Practice => {
            crypto::vault::encrypt_api_key(api_key, master_password.as_bytes())
        }
        OandaEnvironment::Live => {
            crypto::vault::encrypt_live_api_key(api_key, master_password.as_bytes())
        }
    }
    .map_err(|e| format!("Failed to encrypt API key: {}", e))?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    // Read-modify-write the single credential row, preserving the *other*
    // environment's stored blob/account (AC4: existing vaults keep working).
    let existing = local_get_credential(local.clone())?;
    let device_id = match &existing {
        Some(c) => c.device_id.clone(),
        None => state
            .device_manager
            .get_or_create_device_id()
            .map_err(|e| format!("Failed to resolve device id: {}", e))?,
    };

    let mut row = existing.unwrap_or(LocalCredential {
        id: device_id.clone(),
        device_id,
        practice_blob: None,
        practice_account_id: None,
        live_blob: None,
        live_account_id: None,
        created_at: now,
        updated_at: now,
    });
    match env {
        OandaEnvironment::Practice => {
            row.practice_blob = Some(blob);
            row.practice_account_id = Some(account_id.to_string());
        }
        OandaEnvironment::Live => {
            row.live_blob = Some(blob);
            row.live_account_id = Some(account_id.to_string());
        }
    }
    row.updated_at = now;
    local_save_credential(local, row)?;

    // Reflect the new key in the in-memory vault when it is already unlocked so
    // the running session picks it up without a re-unlock. (When locked, the
    // key is loaded on the next `unlock_vault`.)
    {
        let mut credential_vault = state.credential_vault.lock().await;
        if let Some(vault) = credential_vault.as_mut() {
            match env {
                OandaEnvironment::Practice => vault.set_practice_api_key(api_key.to_string()),
                OandaEnvironment::Live => vault.set_live_api_key(api_key.to_string()),
            }
        }
    }

    info!("Saved OANDA {:?} credentials to the encrypted vault", env);

    Ok("Credentials saved to the encrypted vault".to_string())
}
