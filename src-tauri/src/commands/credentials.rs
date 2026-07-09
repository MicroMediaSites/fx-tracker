//! Credential and crypto management commands.
//!
//! Handles master password, vault unlock, and API key management.

use serde::Serialize;
use tauri::State;
use tracing::info;

use crate::AppState;
use candlesight_lib::{
    crypto::{self, PasswordStrength},
    config::OandaEnvironment,
    oanda::{OandaClient, endpoints, PriceStreamer},
};

/// Response type for password strength check
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PasswordStrengthResponse {
    pub score: u8,
    pub feedback: Vec<String>,
    pub is_compromised: bool,
    pub breach_count: u64,
    pub meets_requirements: bool,
}

impl From<PasswordStrength> for PasswordStrengthResponse {
    fn from(s: PasswordStrength) -> Self {
        Self {
            score: s.score,
            feedback: s.feedback,
            is_compromised: s.is_compromised,
            breach_count: s.breach_count,
            meets_requirements: s.meets_requirements,
        }
    }
}

/// Check password strength (local validation only - fast)
#[tauri::command]
pub fn check_password_strength_local(password: String) -> PasswordStrengthResponse {
    crypto::check_password_local(&password).into()
}

/// Check password strength including HIBP breach database (async, makes network call)
#[tauri::command]
pub async fn check_password_strength(password: String) -> Result<PasswordStrengthResponse, String> {
    crypto::check_password(&password)
        .await
        .map(Into::into)
        .map_err(|e| e.to_string())
}

/// Get the unique device ID for this installation
#[tauri::command]
pub fn get_device_id(state: State<'_, AppState>) -> Result<String, String> {
    state.device_manager
        .get_or_create_device_id()
        .map_err(|e| e.to_string())
}

/// Check if rate limited (returns wait time in seconds if limited)
#[tauri::command]
pub async fn check_unlock_rate_limit(state: State<'_, AppState>) -> Result<Option<u64>, String> {
    let limiter = state.rate_limiter.lock().await;
    match limiter.check() {
        Ok(()) => Ok(None),
        Err(duration) => Ok(Some(duration.as_secs())),
    }
}

/// Get rate limit status message
#[tauri::command]
pub async fn get_rate_limit_status(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let limiter = state.rate_limiter.lock().await;
    Ok(limiter.status_message())
}

/// Validate credentials against OANDA by attempting to fetch account info
/// Returns Ok(()) if valid, Err with ACCOUNT_NOT_AVAILABLE if invalid
async fn validate_oanda_credentials(
    api_key: &str,
    account_id: &str,
    environment: OandaEnvironment,
) -> Result<(), String> {
    let client = OandaClient::with_credentials(api_key, account_id, environment)
        .map_err(|_| "ACCOUNT_NOT_AVAILABLE")?;

    // Try to fetch account info - this validates both the API key and account ID
    endpoints::get_account(&client)
        .await
        .map_err(|_| "ACCOUNT_NOT_AVAILABLE".to_string())?;

    Ok(())
}

/// Encrypt API key with master password
/// Account IDs are stored in Zero (not encrypted) since they're not secrets
/// Returns the encrypted API key blob for storage
#[tauri::command]
pub async fn encrypt_credentials(
    master_password: String,
    api_key: String,
    account_id: String,
    environment: Option<String>, // "practice" or "live", defaults to practice
) -> Result<String, String> {
    // Determine which environment to validate against
    let env = match environment.as_deref() {
        Some("live") => OandaEnvironment::Live,
        _ => OandaEnvironment::Practice,
    };

    // Validate credentials against OANDA before storing
    validate_oanda_credentials(&api_key, &account_id, env).await?;

    // Encrypt just the API key
    crypto::vault::encrypt_api_key(&api_key, master_password.as_bytes())
        .map_err(|e| e.to_string())
}

/// Decrypt API key and load into memory vault
/// Account ID comes from Zero (passed by frontend), not encrypted
/// Decrypts both practice and live API keys (if live_blob provided)
/// Returns success/failure - API keys stay in Rust memory, never sent to JS
#[tauri::command]
pub async fn unlock_vault(
    master_password: String,
    practice_blob: String,
    practice_account_id: String,
    live_blob: Option<String>,
    live_account_id: Option<String>,
    environment: Option<String>, // "practice" or "live", defaults to practice
    use_practice_url: Option<bool>, // Dev-only: use practice URL even when environment is "live"
    state: State<'_, AppState>,
) -> Result<bool, String> {
    info!("[unlock_vault] Starting unlock - live_blob: {}, live_account: {}, environment: {:?}, use_practice_url: {:?}",
        live_blob.is_some(),
        if live_account_id.is_some() { "[PRESENT]" } else { "[NONE]" },
        environment,
        use_practice_url
    );

    // Check rate limit first
    {
        let limiter = state.rate_limiter.lock().await;
        if let Err(duration) = limiter.check() {
            return Err(format!(
                "Rate limited. Please wait {} seconds.",
                duration.as_secs()
            ));
        }
    }

    // Attempt to decrypt practice blob (required)
    let practice_api_key = match crypto::vault::decrypt_api_key(&practice_blob, master_password.as_bytes()) {
        Ok(key) => key,
        Err(e) => {
            // Failure - record it
            {
                let mut limiter = state.rate_limiter.lock().await;
                limiter.record_failure().map_err(|e| e.to_string())?;
            }
            return Err(e.to_string());
        }
    };

    // Optionally decrypt live blob if provided
    let live_api_key = if let Some(ref blob) = live_blob {
        match crypto::vault::decrypt_live_api_key(blob, master_password.as_bytes()) {
            Ok(key) => Some(key),
            Err(e) => {
                // Live decryption failed - but don't fail completely
                // Just log and continue without live key
                info!("Live blob decryption failed, continuing without live key: {}", e);
                None
            }
        }
    } else {
        None
    };

    // Success - record it
    {
        let mut limiter = state.rate_limiter.lock().await;
        limiter.record_success().map_err(|e| e.to_string())?;
    }

    // Determine requested environment
    let requested_env = match environment.as_deref() {
        Some("live") => OandaEnvironment::Live,
        _ => OandaEnvironment::Practice,
    };

    // Dev mode: use practice URL even when requesting "live"
    let oanda_env = if use_practice_url.unwrap_or(false) && requested_env == OandaEnvironment::Live {
        info!("[Dev] Using practice URL for Live environment");
        OandaEnvironment::Practice
    } else {
        requested_env
    };

    // Get the appropriate API key and account ID for the REQUESTED environment
    // (not oanda_env - we use the "live" slot credentials even when using practice URL)
    let (api_key, account_id) = match requested_env {
        OandaEnvironment::Live => {
            // For live, we need both live API key and live account ID
            match (&live_api_key, &live_account_id) {
                (Some(key), Some(acc_id)) => (key.clone(), acc_id.clone()),
                _ => {
                    // Fall back to practice if live not available
                    info!("Live credentials not available, falling back to practice");
                    (practice_api_key.clone(), practice_account_id.clone())
                }
            }
        }
        OandaEnvironment::Practice => (practice_api_key.clone(), practice_account_id.clone()),
    };

    // Update the OANDA client with decrypted API key and account ID
    let new_client = OandaClient::with_credentials(
        &api_key,
        &account_id,
        oanda_env,
    ).map_err(|e| e.to_string())?;

    {
        let mut client = state.client.write().await;
        *client = new_client;
    }

    // Update the PriceStreamer with the new credentials
    {
        let mut streamer = state.streamer.lock().await;
        let new_streamer = PriceStreamer::new(&api_key, &account_id, &oanda_env);
        *streamer = new_streamer;
        info!("[unlock_vault] PriceStreamer updated with new credentials");
    }

    // Store the vault with both API keys
    {
        let vault = crypto::vault::create_vault_with_both_keys(practice_api_key, live_api_key);
        info!("[unlock_vault] Vault created - has_practice: {}, has_live: {}",
            vault.has_practice(),
            vault.has_live()
        );
        let mut credential_vault = state.credential_vault.lock().await;
        *credential_vault = Some(vault);
    }

    Ok(true)
}

/// Lock the vault (clear credentials from memory)
#[tauri::command]
pub async fn lock_vault(state: State<'_, AppState>) -> Result<(), String> {
    let mut credential_vault = state.credential_vault.lock().await;
    *credential_vault = None;
    Ok(())
}

/// Check if the vault is currently unlocked
#[tauri::command]
pub async fn is_vault_unlocked(state: State<'_, AppState>) -> Result<bool, String> {
    let credential_vault = state.credential_vault.lock().await;
    Ok(credential_vault.is_some())
}

/// Check if practice credentials are available
#[tauri::command]
pub async fn has_practice_credentials(state: State<'_, AppState>) -> Result<bool, String> {
    let credential_vault = state.credential_vault.lock().await;
    Ok(credential_vault.as_ref().map(|v| v.has_practice()).unwrap_or(false))
}

/// Check if live credentials are available
#[tauri::command]
pub async fn has_live_credentials(state: State<'_, AppState>) -> Result<bool, String> {
    let credential_vault = state.credential_vault.lock().await;
    Ok(credential_vault.as_ref().map(|v| v.has_live()).unwrap_or(false))
}

/// Clear live credentials from the in-memory vault
/// Frontend is responsible for updating Zero to null out live_blob and live_account_id
#[tauri::command]
pub async fn clear_live_credentials(state: State<'_, AppState>) -> Result<(), String> {
    let mut credential_vault = state.credential_vault.lock().await;
    if let Some(vault) = credential_vault.as_mut() {
        vault.clear_live();
    }
    Ok(())
}

/// Add live credentials - validates, encrypts, and returns blob for storage
/// Live API key is separate from practice API key
#[tauri::command]
pub async fn add_live_credentials(
    master_password: String,
    live_api_key: String,
    live_account_id: String,
    practice_blob: String, // Used to verify master password
    validate_as_practice: Option<bool>, // Dev-only: validate against practice instead of live
    state: State<'_, AppState>,
) -> Result<String, String> {
    // Verify master password by decrypting practice blob
    crypto::vault::decrypt_api_key(&practice_blob, master_password.as_bytes())
        .map_err(|_| "Invalid master password")?;

    // Validate the live API key against OANDA
    // Dev mode can validate against practice for testing
    let env = if validate_as_practice.unwrap_or(false) {
        OandaEnvironment::Practice
    } else {
        OandaEnvironment::Live
    };
    validate_oanda_credentials(&live_api_key, &live_account_id, env).await?;

    // Encrypt the live API key with master password (uses Live subkey context)
    let live_blob = crypto::vault::encrypt_live_api_key(&live_api_key, master_password.as_bytes())
        .map_err(|e| format!("Failed to encrypt live API key: {}", e))?;

    // Update in-memory vault with the live API key
    {
        let mut credential_vault = state.credential_vault.lock().await;
        if let Some(vault) = credential_vault.as_mut() {
            vault.set_live_api_key(live_api_key);
        }
    }

    info!("Added live credentials successfully");
    Ok(live_blob)
}

/// Update API key for a specific environment - validates against OANDA and re-encrypts
/// Returns new encrypted blob for frontend to store in Zero
#[tauri::command]
pub async fn update_api_key_with_vault(
    master_password: String,
    new_api_key: String,
    account_id: String,
    current_blob: String,
    environment: Option<String>, // "practice" or "live", defaults to practice
    validate_as_practice: Option<bool>, // Dev-only: validate against practice instead of live
    state: State<'_, AppState>,
) -> Result<String, String> {
    let is_live = environment.as_deref() == Some("live");

    // Verify master password by decrypting the provided blob
    if is_live {
        crypto::vault::decrypt_live_api_key(&current_blob, master_password.as_bytes())
            .map_err(|_| "Invalid master password")?;
    } else {
        crypto::vault::decrypt_api_key(&current_blob, master_password.as_bytes())
            .map_err(|_| "Invalid master password")?;
    }

    // Validate new API key against OANDA
    // Dev mode can override to use practice URL for testing
    let validation_env = if validate_as_practice.unwrap_or(false) {
        OandaEnvironment::Practice
    } else if is_live {
        OandaEnvironment::Live
    } else {
        OandaEnvironment::Practice
    };
    validate_oanda_credentials(&new_api_key, &account_id, validation_env).await?;

    // Encrypt new API key with master password (using appropriate context)
    let new_blob = if is_live {
        crypto::vault::encrypt_live_api_key(&new_api_key, master_password.as_bytes())
            .map_err(|e| format!("Failed to encrypt live API key: {}", e))?
    } else {
        crypto::vault::encrypt_api_key(&new_api_key, master_password.as_bytes())
            .map_err(|e| format!("Failed to encrypt practice API key: {}", e))?
    };

    // Update in-memory vault
    {
        let mut credential_vault = state.credential_vault.lock().await;
        if let Some(vault) = credential_vault.as_mut() {
            if is_live {
                vault.set_live_api_key(new_api_key.clone());
            } else {
                vault.set_practice_api_key(new_api_key.clone());
            }
        }
    }

    // Update OANDA client if we're on the environment being updated
    // Use actual target environment, not validation environment
    let target_env = if is_live { OandaEnvironment::Live } else { OandaEnvironment::Practice };
    {
        let current_env = {
            let client = state.client.read().await;
            client.environment()
        };

        if current_env == target_env {
            let new_client = OandaClient::with_credentials(&new_api_key, &account_id, target_env)
                .map_err(|e| format!("Failed to update client: {}", e))?;
            let mut client = state.client.write().await;
            *client = new_client;
            info!("Updated OANDA client with new {:?} API key", target_env);
        }
    }

    Ok(new_blob)
}

/// Update API key - requires re-encryption with master password
/// Returns new encrypted blob
#[tauri::command]
pub async fn update_api_key(
    master_password: String,
    current_blob: String,
    new_api_key: String,
    account_id: String,
    environment: Option<String>, // "practice" or "live", defaults to practice
    state: State<'_, AppState>,
) -> Result<String, String> {
    // Determine environment
    let env = match environment.as_deref() {
        Some("live") => OandaEnvironment::Live,
        _ => OandaEnvironment::Practice,
    };

    // Validate new credentials against OANDA
    validate_oanda_credentials(&new_api_key, &account_id, env).await?;

    // Verify master password by decrypting current blob
    crypto::vault::decrypt_api_key(&current_blob, master_password.as_bytes())
        .map_err(|_| "Invalid master password")?;

    // Encrypt new API key
    let new_blob = crypto::vault::encrypt_api_key(&new_api_key, master_password.as_bytes())
        .map_err(|e| format!("Failed to encrypt credentials: {}", e))?;

    // Update in-memory vault
    {
        let mut credential_vault = state.credential_vault.lock().await;
        if let Some(vault) = credential_vault.as_mut() {
            vault.set_api_key(new_api_key.clone());
        }
    }

    // Update OANDA client
    {
        let new_client = OandaClient::with_credentials(&new_api_key, &account_id, env)
            .map_err(|e| format!("Failed to update client: {}", e))?;
        let mut client = state.client.write().await;
        *client = new_client;
        info!("Updated OANDA client with new API key");
    }

    Ok(new_blob)
}

/// Validate account ID with existing API key
/// No re-encryption needed - just validates and frontend stores to Zero
#[tauri::command]
pub async fn validate_account_id(
    account_id: String,
    environment: String, // "practice" or "live" - determines which API key slot to use
    validate_as_practice: Option<bool>, // Dev-only: validate against practice URL instead of live
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Which API key to use (based on environment param - the slot)
    let api_key = {
        let credential_vault = state.credential_vault.lock().await;
        let vault = credential_vault.as_ref()
            .ok_or("Vault not unlocked")?;

        info!("[validate_account_id] Vault state - has_practice: {}, has_live: {}, environment: {}",
            vault.has_practice(),
            vault.has_live(),
            environment
        );

        match environment.as_str() {
            "live" => {
                vault.get_live_api_key()
                    .ok_or("No live API key in vault - add live credentials first")?
                    .to_string()
            }
            _ => {
                vault.get_practice_api_key()
                    .ok_or("No practice API key in vault")?
                    .to_string()
            }
        }
    };

    // Which URL to hit (validate_as_practice can override to use practice URL for live slot)
    let url_env = if validate_as_practice.unwrap_or(false) {
        info!("[validate_account_id] Dev mode: using practice URL for validation");
        OandaEnvironment::Practice
    } else if environment == "live" {
        OandaEnvironment::Live
    } else {
        OandaEnvironment::Practice
    };

    // Mask account ID for security (show only last 4 chars)
    let masked_account = if account_id.len() > 4 {
        format!("***{}", &account_id[account_id.len()-4..])
    } else {
        "****".to_string()
    };
    info!("[validate_account_id] Validating account {} with {} slot key against {:?} URL",
        masked_account, environment, url_env);
    validate_oanda_credentials(&api_key, &account_id, url_env).await?;

    Ok(())
}
