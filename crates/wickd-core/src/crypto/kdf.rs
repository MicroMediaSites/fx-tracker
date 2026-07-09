use argon2::{Argon2, Algorithm, Params, Version};
use hkdf::Hkdf;
use sha2::Sha256;
use crate::error::{Error, Result};
use super::types::{KdfParams, MasterKey, DerivedKey, SubkeyContext, KEY_LEN, SALT_LEN};

pub fn derive_master_key(password: &[u8], salt: &[u8], params: &KdfParams) -> Result<MasterKey> {
    if salt.len() != SALT_LEN {
        return Err(Error::Crypto(format!(
            "Invalid salt length: expected {}, got {}",
            SALT_LEN,
            salt.len()
        )));
    }

    let argon2_params = Params::new(
        params.memory_kib,
        params.iterations,
        params.parallelism,
        Some(KEY_LEN),
    )
    .map_err(|e| Error::Crypto(format!("Invalid Argon2 parameters: {}", e)))?;

    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon2_params);

    let mut key = [0u8; KEY_LEN];
    argon2
        .hash_password_into(password, salt, &mut key)
        .map_err(|e| Error::Crypto(format!("Key derivation failed: {}", e)))?;

    Ok(MasterKey::new(key))
}

pub fn derive_subkey(master_key: &MasterKey, context: SubkeyContext) -> Result<DerivedKey> {
    let hkdf = Hkdf::<Sha256>::new(None, master_key.as_bytes());

    let mut subkey = [0u8; KEY_LEN];
    hkdf.expand(context.as_bytes(), &mut subkey)
        .map_err(|e| Error::Crypto(format!("Subkey derivation failed: {}", e)))?;

    Ok(DerivedKey::new(subkey))
}

pub fn generate_salt() -> [u8; SALT_LEN] {
    use rand::RngCore;
    let mut salt = [0u8; SALT_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    salt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_master_key_deterministic() {
        let password = b"test-password-123456";
        let salt = [0u8; SALT_LEN];
        let params = KdfParams::default();

        let key1 = derive_master_key(password, &salt, &params).unwrap();
        let key2 = derive_master_key(password, &salt, &params).unwrap();

        assert_eq!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn test_different_passwords_produce_different_keys() {
        let salt = [0u8; SALT_LEN];
        let params = KdfParams::default();

        let key1 = derive_master_key(b"password1-123456", &salt, &params).unwrap();
        let key2 = derive_master_key(b"password2-123456", &salt, &params).unwrap();

        assert_ne!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn test_different_salts_produce_different_keys() {
        let password = b"test-password-123456";
        let params = KdfParams::default();

        let salt1 = [0u8; SALT_LEN];
        let salt2 = [1u8; SALT_LEN];

        let key1 = derive_master_key(password, &salt1, &params).unwrap();
        let key2 = derive_master_key(password, &salt2, &params).unwrap();

        assert_ne!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn test_invalid_salt_length_fails() {
        let password = b"test-password-123456";
        let params = KdfParams::default();

        let short_salt = [0u8; 8];
        let result = derive_master_key(password, &short_salt, &params);

        assert!(result.is_err());
    }

    #[test]
    fn test_derive_subkey_deterministic() {
        let password = b"test-password-123456";
        let salt = [0u8; SALT_LEN];
        let params = KdfParams::default();

        let master = derive_master_key(password, &salt, &params).unwrap();

        let subkey1 = derive_subkey(&master, SubkeyContext::Practice).unwrap();
        let subkey2 = derive_subkey(&master, SubkeyContext::Practice).unwrap();

        assert_eq!(subkey1.as_bytes(), subkey2.as_bytes());
    }

    #[test]
    fn test_different_contexts_produce_different_subkeys() {
        let password = b"test-password-123456";
        let salt = [0u8; SALT_LEN];
        let params = KdfParams::default();

        let master = derive_master_key(password, &salt, &params).unwrap();

        let practice_key = derive_subkey(&master, SubkeyContext::Practice).unwrap();
        let live_key = derive_subkey(&master, SubkeyContext::Live).unwrap();
        let hmac_key = derive_subkey(&master, SubkeyContext::Hmac).unwrap();
        let trading_key = derive_subkey(&master, SubkeyContext::Trading).unwrap();

        assert_ne!(practice_key.as_bytes(), live_key.as_bytes());
        assert_ne!(practice_key.as_bytes(), hmac_key.as_bytes());
        assert_ne!(practice_key.as_bytes(), trading_key.as_bytes());
        assert_ne!(live_key.as_bytes(), hmac_key.as_bytes());
        assert_ne!(live_key.as_bytes(), trading_key.as_bytes());
        assert_ne!(hmac_key.as_bytes(), trading_key.as_bytes());
    }

    #[test]
    fn test_generate_salt_unique() {
        let salt1 = generate_salt();
        let salt2 = generate_salt();

        assert_ne!(salt1, salt2);
    }

    #[test]
    fn test_generate_salt_correct_length() {
        let salt = generate_salt();
        assert_eq!(salt.len(), SALT_LEN);
    }
}
