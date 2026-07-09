use serde::{Deserialize, Serialize};
use secrecy::{ExposeSecret, Secret};
use zeroize::{Zeroize, ZeroizeOnDrop};
use crate::error::{Error, Result};
use super::types::{ProtectedBlob, SubkeyContext};
use super::encryption::{encrypt_with_password, decrypt_with_password};

/// Legacy credential format (for backward compatibility)
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct OandaCredentials {
    pub api_key: String,
    pub account_id: String,
}

impl OandaCredentials {
    pub fn new(api_key: String, account_id: String) -> Self {
        Self { api_key, account_id }
    }
}

/// Simplified format - just the API key
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
struct ApiKeyOnly {
    api_key: String,
}

/// Credential vault that stores API keys for both environments
/// Account IDs are stored in Zero (not sensitive)
#[derive(Default)]
pub struct CredentialVault {
    practice_api_key: Option<Secret<String>>,
    live_api_key: Option<Secret<String>>,
}

impl CredentialVault {
    pub fn new() -> Self {
        Self::default()
    }

    // Practice API key methods
    pub fn set_practice_api_key(&mut self, api_key: String) {
        self.practice_api_key = Some(Secret::new(api_key));
    }

    pub fn has_practice_api_key(&self) -> bool {
        self.practice_api_key.is_some()
    }

    pub fn get_practice_api_key(&self) -> Option<&str> {
        self.practice_api_key.as_ref().map(|s| s.expose_secret().as_str())
    }

    // Live API key methods
    pub fn set_live_api_key(&mut self, api_key: String) {
        self.live_api_key = Some(Secret::new(api_key));
    }

    pub fn has_live_api_key(&self) -> bool {
        self.live_api_key.is_some()
    }

    pub fn get_live_api_key(&self) -> Option<&str> {
        self.live_api_key.as_ref().map(|s| s.expose_secret().as_str())
    }

    pub fn clear_live_api_key(&mut self) {
        self.live_api_key = None;
    }

    pub fn clear(&mut self) {
        self.practice_api_key = None;
        self.live_api_key = None;
    }

    // Legacy compatibility methods

    /// Legacy alias - same as set_practice_api_key
    pub fn set_api_key(&mut self, api_key: String) {
        self.set_practice_api_key(api_key);
    }

    /// Legacy alias - same as has_practice_api_key
    pub fn has_api_key(&self) -> bool {
        self.has_practice_api_key()
    }

    /// Legacy alias - same as get_practice_api_key
    pub fn get_api_key(&self) -> Option<&str> {
        self.get_practice_api_key()
    }

    pub fn has_practice(&self) -> bool {
        self.has_practice_api_key()
    }

    pub fn has_live(&self) -> bool {
        self.has_live_api_key()
    }

    /// Legacy method - returns a pseudo OandaCredentials with empty account_id
    /// Callers should get account_id from Zero instead
    pub fn get_practice(&self) -> Option<OandaCredentials> {
        self.practice_api_key.as_ref().map(|key| OandaCredentials {
            api_key: key.expose_secret().clone(),
            account_id: String::new(), // Account ID comes from Zero now
        })
    }

    /// Legacy method - returns a pseudo OandaCredentials with empty account_id
    /// Callers should get account_id from Zero instead
    pub fn get_live(&self) -> Option<OandaCredentials> {
        self.live_api_key.as_ref().map(|key| OandaCredentials {
            api_key: key.expose_secret().clone(),
            account_id: String::new(), // Account ID comes from Zero now
        })
    }

    pub fn clear_live(&mut self) {
        self.clear_live_api_key();
    }
}

impl Drop for CredentialVault {
    fn drop(&mut self) {
        self.clear();
    }
}

/// Encrypt an API key for the practice environment
pub fn encrypt_api_key(
    api_key: &str,
    password: &[u8],
) -> Result<String> {
    encrypt_api_key_for_env(api_key, password, SubkeyContext::Practice)
}

/// Encrypt an API key for the live environment
pub fn encrypt_live_api_key(
    api_key: &str,
    password: &[u8],
) -> Result<String> {
    encrypt_api_key_for_env(api_key, password, SubkeyContext::Live)
}

/// Encrypt an API key for a specific environment
fn encrypt_api_key_for_env(
    api_key: &str,
    password: &[u8],
    context: SubkeyContext,
) -> Result<String> {
    let data = ApiKeyOnly { api_key: api_key.to_string() };
    let json = serde_json::to_vec(&data)
        .map_err(|e| Error::Crypto(format!("Failed to serialize API key: {}", e)))?;
    let protected = encrypt_with_password(&json, password, context)?;
    protected.to_base64().map_err(|e| Error::Crypto(e.to_string()))
}

/// Decrypt API key from practice blob
pub fn decrypt_api_key(
    blob: &str,
    password: &[u8],
) -> Result<String> {
    decrypt_api_key_for_env(blob, password, SubkeyContext::Practice)
}

/// Decrypt API key from live blob
pub fn decrypt_live_api_key(
    blob: &str,
    password: &[u8],
) -> Result<String> {
    decrypt_api_key_for_env(blob, password, SubkeyContext::Live)
}

/// Decrypt API key from blob for a specific environment
/// Supports both new format (just API key) and legacy format (api_key + account_id)
fn decrypt_api_key_for_env(
    blob: &str,
    password: &[u8],
    context: SubkeyContext,
) -> Result<String> {
    let protected = ProtectedBlob::from_base64(blob)
        .map_err(|e| Error::Crypto(e.to_string()))?;
    let mut decrypted = decrypt_with_password(&protected, password, context)?;

    // Try new format first (just API key)
    if let Ok(data) = serde_json::from_slice::<ApiKeyOnly>(&decrypted) {
        let api_key = data.api_key.clone();
        decrypted.zeroize();
        return Ok(api_key);
    }

    // Fall back to legacy format (api_key + account_id)
    if let Ok(data) = serde_json::from_slice::<OandaCredentials>(&decrypted) {
        let api_key = data.api_key.clone();
        decrypted.zeroize();
        return Ok(api_key);
    }

    decrypted.zeroize();
    Err(Error::Crypto("Failed to parse credential blob".to_string()))
}

/// Create a vault from a decrypted practice API key
pub fn create_vault_with_api_key(api_key: String) -> CredentialVault {
    let mut vault = CredentialVault::new();
    vault.set_practice_api_key(api_key);
    vault
}

/// Create a vault with both practice and live API keys
pub fn create_vault_with_both_keys(practice_api_key: String, live_api_key: Option<String>) -> CredentialVault {
    let mut vault = CredentialVault::new();
    vault.set_practice_api_key(practice_api_key);
    if let Some(live_key) = live_api_key {
        vault.set_live_api_key(live_key);
    }
    vault
}

// Legacy functions for backward compatibility

pub fn encrypt_credentials(
    vault: &CredentialVault,
    password: &[u8],
) -> Result<(Option<String>, Option<String>)> {
    let practice_blob = if let Some(api_key) = vault.get_practice_api_key() {
        Some(encrypt_api_key(api_key, password)?)
    } else {
        None
    };

    let live_blob = if let Some(api_key) = vault.get_live_api_key() {
        Some(encrypt_live_api_key(api_key, password)?)
    } else {
        None
    };

    Ok((practice_blob, live_blob))
}

pub fn decrypt_credentials(
    practice_blob: Option<&str>,
    live_blob: Option<&str>,
    password: &[u8],
) -> Result<CredentialVault> {
    let mut vault = CredentialVault::new();

    if let Some(blob_str) = practice_blob {
        let api_key = decrypt_api_key(blob_str, password)?;
        vault.set_practice_api_key(api_key);
    }

    if let Some(blob_str) = live_blob {
        let api_key = decrypt_live_api_key(blob_str, password)?;
        vault.set_live_api_key(api_key);
    }

    Ok(vault)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vault_encrypt_decrypt_practice_api_key() {
        let password = b"master-password-16ch";
        let api_key = "test-api-key-12345";

        let blob = encrypt_api_key(api_key, password).unwrap();
        let decrypted = decrypt_api_key(&blob, password).unwrap();

        assert_eq!(decrypted, api_key);
    }

    #[test]
    fn test_vault_encrypt_decrypt_live_api_key() {
        let password = b"master-password-16ch";
        let api_key = "live-api-key-67890";

        let blob = encrypt_live_api_key(api_key, password).unwrap();
        let decrypted = decrypt_live_api_key(&blob, password).unwrap();

        assert_eq!(decrypted, api_key);
    }

    #[test]
    fn test_vault_practice_and_live_keys_different_encryption() {
        let password = b"master-password-16ch";
        let api_key = "same-api-key";

        let practice_blob = encrypt_api_key(api_key, password).unwrap();
        let live_blob = encrypt_live_api_key(api_key, password).unwrap();

        // Same key encrypted for different contexts should produce different blobs
        assert_ne!(practice_blob, live_blob);

        // But both should decrypt correctly
        assert_eq!(decrypt_api_key(&practice_blob, password).unwrap(), api_key);
        assert_eq!(decrypt_live_api_key(&live_blob, password).unwrap(), api_key);
    }

    #[test]
    fn test_vault_cannot_decrypt_with_wrong_context() {
        let password = b"master-password-16ch";
        let api_key = "test-api-key";

        // Encrypt as practice
        let practice_blob = encrypt_api_key(api_key, password).unwrap();

        // Try to decrypt as live - should fail
        let result = decrypt_live_api_key(&practice_blob, password);
        assert!(result.is_err());
    }

    #[test]
    fn test_vault_backward_compat_legacy_format() {
        // Simulate a legacy blob with api_key + account_id
        let password = b"master-password-16ch";
        let legacy_data = OandaCredentials::new(
            "legacy-api-key".to_string(),
            "101-001-12345-001".to_string(),
        );
        let json = serde_json::to_vec(&legacy_data).unwrap();
        let protected = encrypt_with_password(&json, password, SubkeyContext::Practice).unwrap();
        let legacy_blob = protected.to_base64().unwrap();

        // Should be able to decrypt and extract just the API key
        let api_key = decrypt_api_key(&legacy_blob, password).unwrap();
        assert_eq!(api_key, "legacy-api-key");
    }

    #[test]
    fn test_vault_wrong_password_fails() {
        let password = b"correct-password-16!";
        let wrong_password = b"wrong-password-16!!";

        let blob = encrypt_api_key("test-api-key", password).unwrap();
        let result = decrypt_api_key(&blob, wrong_password);

        assert!(result.is_err());
    }

    #[test]
    fn test_vault_has_both_keys() {
        let mut vault = CredentialVault::new();
        assert!(!vault.has_practice_api_key());
        assert!(!vault.has_live_api_key());

        vault.set_practice_api_key("practice-key".to_string());
        assert!(vault.has_practice_api_key());
        assert!(!vault.has_live_api_key());

        vault.set_live_api_key("live-key".to_string());
        assert!(vault.has_practice_api_key());
        assert!(vault.has_live_api_key());
    }

    #[test]
    fn test_vault_clear() {
        let mut vault = CredentialVault::new();
        vault.set_practice_api_key("practice-key".to_string());
        vault.set_live_api_key("live-key".to_string());

        vault.clear();
        assert!(!vault.has_practice_api_key());
        assert!(!vault.has_live_api_key());
    }

    #[test]
    fn test_vault_clear_live_only() {
        let mut vault = CredentialVault::new();
        vault.set_practice_api_key("practice-key".to_string());
        vault.set_live_api_key("live-key".to_string());

        vault.clear_live_api_key();
        assert!(vault.has_practice_api_key());
        assert!(!vault.has_live_api_key());
    }

    #[test]
    fn test_legacy_compat_methods() {
        let mut vault = CredentialVault::new();
        vault.set_practice_api_key("test-api-key".to_string());

        // Legacy methods should work
        assert!(vault.has_practice());
        assert!(!vault.has_live());

        let practice = vault.get_practice().unwrap();
        assert_eq!(practice.api_key, "test-api-key");
        assert!(practice.account_id.is_empty());
    }

    #[test]
    fn test_encrypt_decrypt_credentials_roundtrip() {
        let mut vault = CredentialVault::new();
        vault.set_practice_api_key("practice-api-key".to_string());
        vault.set_live_api_key("live-api-key".to_string());

        let password = b"master-password-16ch";
        let (practice_blob, live_blob) = encrypt_credentials(&vault, password).unwrap();

        assert!(practice_blob.is_some());
        assert!(live_blob.is_some());

        let decrypted = decrypt_credentials(
            practice_blob.as_deref(),
            live_blob.as_deref(),
            password,
        ).unwrap();

        assert_eq!(decrypted.get_practice_api_key().unwrap(), "practice-api-key");
        assert_eq!(decrypted.get_live_api_key().unwrap(), "live-api-key");
    }

    #[test]
    fn test_create_vault_with_both_keys() {
        let vault = create_vault_with_both_keys(
            "practice-key".to_string(),
            Some("live-key".to_string()),
        );

        assert_eq!(vault.get_practice_api_key().unwrap(), "practice-key");
        assert_eq!(vault.get_live_api_key().unwrap(), "live-key");
    }

    #[test]
    fn test_create_vault_with_practice_only() {
        let vault = create_vault_with_both_keys(
            "practice-key".to_string(),
            None,
        );

        assert_eq!(vault.get_practice_api_key().unwrap(), "practice-key");
        assert!(!vault.has_live_api_key());
    }
}
