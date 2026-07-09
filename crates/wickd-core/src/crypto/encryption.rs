use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use crate::error::{Error, Result};
use super::types::{
    DerivedKey, EncryptedBlob, KdfParams, ProtectedBlob, SubkeyContext,
    BLOB_VERSION, HMAC_FORMAT_VERSION, KEY_LEN, NONCE_LEN, HMAC_LEN,
    MIN_KDF_MEMORY, MAX_KDF_MEMORY, MIN_KDF_ITERATIONS, MAX_KDF_ITERATIONS,
    MIN_KDF_PARALLELISM, MAX_KDF_PARALLELISM,
};
use super::kdf::{derive_master_key, derive_subkey, generate_salt};

type HmacSha256 = Hmac<Sha256>;

pub fn generate_nonce() -> [u8; NONCE_LEN] {
    use rand::RngCore;
    let mut nonce = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    nonce
}

pub fn encrypt(plaintext: &[u8], key: &DerivedKey) -> Result<EncryptedBlob> {
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes())
        .map_err(|e| Error::Crypto(format!("Failed to create cipher: {}", e)))?;

    let nonce_bytes = generate_nonce();
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| Error::Crypto(format!("Encryption failed: {}", e)))?;

    Ok(EncryptedBlob {
        version: BLOB_VERSION,
        salt: Vec::new(),
        kdf_params: KdfParams::default(),
        nonce: nonce_bytes.to_vec(),
        ciphertext,
    })
}

pub fn decrypt(blob: &EncryptedBlob, key: &DerivedKey) -> Result<Vec<u8>> {
    if blob.nonce.len() != NONCE_LEN {
        return Err(Error::Crypto(format!(
            "Invalid nonce length: expected {}, got {}",
            NONCE_LEN,
            blob.nonce.len()
        )));
    }

    let cipher = Aes256Gcm::new_from_slice(key.as_bytes())
        .map_err(|e| Error::Crypto(format!("Failed to create cipher: {}", e)))?;

    let nonce = Nonce::from_slice(&blob.nonce);

    cipher
        .decrypt(nonce, blob.ciphertext.as_ref())
        .map_err(|_| Error::Auth("Decryption failed: invalid password or corrupted data".to_string()))
}

/// Derive the dedicated MAC key from an HMAC subkey via HKDF-SHA256.
///
/// This extra expansion is preserved from the original scheme so that the legacy
/// (v1) MAC can still be reproduced for back-compat verification. The caller is
/// responsible for zeroizing the returned key after use.
fn derive_mac_key(key: &DerivedKey) -> [u8; KEY_LEN] {
    let hkdf = Hkdf::<Sha256>::new(None, key.as_bytes());
    let mut mac_key = [0u8; KEY_LEN];
    hkdf.expand(b"hmac-key", &mut mac_key)
        .expect("HMAC key derivation should not fail");
    mac_key
}

/// Compute the current (v2) MAC over `data`: `HMAC-SHA256(mac_key, data)`.
pub fn compute_hmac(key: &DerivedKey, data: &[u8]) -> [u8; HMAC_LEN] {
    let mut mac_key = derive_mac_key(key);

    let mut mac = <HmacSha256 as Mac>::new_from_slice(&mac_key)
        .expect("HMAC accepts a key of any length");
    mac.update(data);
    let result = mac.finalize().into_bytes();

    mac_key.iter_mut().for_each(|b| *b = 0);

    let mut out = [0u8; HMAC_LEN];
    out.copy_from_slice(&result);
    out
}

/// Reproduce the legacy (v1) hand-rolled secret-prefix MAC `SHA-256(mac_key ‖ data)`.
///
/// This is length-extension-vulnerable and is retained ONLY to verify vaults
/// written before AGT-664. It is never used to produce new MACs.
fn compute_legacy_mac(key: &DerivedKey, data: &[u8]) -> [u8; HMAC_LEN] {
    use sha2::Digest;

    let mut mac_key = derive_mac_key(key);

    let mut hasher = Sha256::new();
    hasher.update(&mac_key);
    hasher.update(data);
    let result = hasher.finalize();

    mac_key.iter_mut().for_each(|b| *b = 0);

    let mut out = [0u8; HMAC_LEN];
    out.copy_from_slice(&result);
    out
}

/// Verify the MAC over `data` for a blob of the given on-disk `version`.
///
/// - v2+ (`HMAC_FORMAT_VERSION`): verified with a constant-time HMAC-SHA256 check
///   only — there is deliberately no fallback to the legacy MAC, so a v2 blob
///   cannot be downgraded to the weaker scheme.
/// - v1 (and any earlier): verified with a constant-time compare against the
///   legacy secret-prefix MAC, so existing vaults still unlock.
pub fn verify_hmac(key: &DerivedKey, data: &[u8], expected: &[u8], version: u8) -> bool {
    if expected.len() != HMAC_LEN {
        return false;
    }

    if version >= HMAC_FORMAT_VERSION {
        let mut mac_key = derive_mac_key(key);
        let verified = match <HmacSha256 as Mac>::new_from_slice(&mac_key) {
            Ok(mut mac) => {
                mac.update(data);
                // Mac::verify_slice is a constant-time comparison.
                mac.verify_slice(expected).is_ok()
            }
            Err(_) => false,
        };
        mac_key.iter_mut().for_each(|b| *b = 0);
        verified
    } else {
        let legacy = compute_legacy_mac(key, data);
        legacy.ct_eq(expected).into()
    }
}

/// Reject KDF parameters read from an untrusted blob that fall outside sane
/// bounds, BEFORE any Argon2 memory is allocated. This prevents a hostile blob
/// from requesting, e.g., `memory_kib = u32::MAX` to exhaust memory on unlock.
/// Rejecting (rather than silently clamping) is intentional: clamping would still
/// allocate up to the ceiling and would also silently derive a different key.
fn validate_kdf_params(params: &KdfParams) -> Result<()> {
    if params.memory_kib < MIN_KDF_MEMORY || params.memory_kib > MAX_KDF_MEMORY {
        return Err(Error::Crypto(format!(
            "KDF memory_kib {} out of bounds [{}, {}]",
            params.memory_kib, MIN_KDF_MEMORY, MAX_KDF_MEMORY
        )));
    }
    if params.iterations < MIN_KDF_ITERATIONS || params.iterations > MAX_KDF_ITERATIONS {
        return Err(Error::Crypto(format!(
            "KDF iterations {} out of bounds [{}, {}]",
            params.iterations, MIN_KDF_ITERATIONS, MAX_KDF_ITERATIONS
        )));
    }
    if params.parallelism < MIN_KDF_PARALLELISM || params.parallelism > MAX_KDF_PARALLELISM {
        return Err(Error::Crypto(format!(
            "KDF parallelism {} out of bounds [{}, {}]",
            params.parallelism, MIN_KDF_PARALLELISM, MAX_KDF_PARALLELISM
        )));
    }
    Ok(())
}

pub fn encrypt_with_password(
    plaintext: &[u8],
    password: &[u8],
    context: SubkeyContext,
) -> Result<ProtectedBlob> {
    let salt = generate_salt();
    let kdf_params = KdfParams::default();

    let master_key = derive_master_key(password, &salt, &kdf_params)?;
    let encryption_key = derive_subkey(&master_key, context)?;
    let hmac_key = derive_subkey(&master_key, SubkeyContext::Hmac)?;

    let mut blob = encrypt(plaintext, &encryption_key)?;
    blob.salt = salt.to_vec();
    blob.kdf_params = kdf_params;

    let blob_json = serde_json::to_vec(&blob)
        .map_err(|e| Error::Crypto(format!("Failed to serialize blob: {}", e)))?;
    let hmac = compute_hmac(&hmac_key, &blob_json);

    Ok(ProtectedBlob {
        encrypted: blob,
        hmac: hmac.to_vec(),
    })
}

pub fn decrypt_with_password(
    protected: &ProtectedBlob,
    password: &[u8],
    context: SubkeyContext,
) -> Result<Vec<u8>> {
    if protected.encrypted.salt.len() != super::types::SALT_LEN {
        return Err(Error::Crypto("Invalid salt length".to_string()));
    }

    // Clamp/validate untrusted KDF params BEFORE deriving the key (Argon2
    // allocates `memory_kib` during derivation), so a hostile blob cannot force a
    // memory-exhaustion allocation on unlock.
    validate_kdf_params(&protected.encrypted.kdf_params)?;

    let master_key = derive_master_key(
        password,
        &protected.encrypted.salt,
        &protected.encrypted.kdf_params,
    )?;
    let hmac_key = derive_subkey(&master_key, SubkeyContext::Hmac)?;

    let blob_json = serde_json::to_vec(&protected.encrypted)
        .map_err(|e| Error::Crypto(format!("Failed to serialize blob: {}", e)))?;

    if !verify_hmac(&hmac_key, &blob_json, &protected.hmac, protected.encrypted.version) {
        return Err(Error::Auth("HMAC verification failed: data may have been tampered".to_string()));
    }

    let encryption_key = derive_subkey(&master_key, context)?;
    decrypt(&protected.encrypted, &encryption_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let password = b"test-password-16chars!";
        let plaintext = b"secret api key data";

        let protected = encrypt_with_password(plaintext, password, SubkeyContext::Practice).unwrap();
        let decrypted = decrypt_with_password(&protected, password, SubkeyContext::Practice).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_wrong_password_fails() {
        let password = b"correct-password-16!";
        let wrong_password = b"wrong-password-16!!";
        let plaintext = b"secret api key data";

        let protected = encrypt_with_password(plaintext, password, SubkeyContext::Practice).unwrap();
        let result = decrypt_with_password(&protected, wrong_password, SubkeyContext::Practice);

        assert!(result.is_err());
    }

    #[test]
    fn test_wrong_context_fails() {
        let password = b"test-password-16chars!";
        let plaintext = b"secret api key data";

        let protected = encrypt_with_password(plaintext, password, SubkeyContext::Practice).unwrap();
        let result = decrypt_with_password(&protected, password, SubkeyContext::Live);

        assert!(result.is_err());
    }

    #[test]
    fn test_tampered_hmac_fails() {
        let password = b"test-password-16chars!";
        let plaintext = b"secret api key data";

        let mut protected = encrypt_with_password(plaintext, password, SubkeyContext::Practice).unwrap();
        protected.hmac[0] ^= 0xFF;

        let result = decrypt_with_password(&protected, password, SubkeyContext::Practice);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("HMAC"));
    }

    #[test]
    fn test_tampered_ciphertext_fails() {
        let password = b"test-password-16chars!";
        let plaintext = b"secret api key data";

        let mut protected = encrypt_with_password(plaintext, password, SubkeyContext::Practice).unwrap();
        if !protected.encrypted.ciphertext.is_empty() {
            protected.encrypted.ciphertext[0] ^= 0xFF;
        }

        let result = decrypt_with_password(&protected, password, SubkeyContext::Practice);
        assert!(result.is_err());
    }

    #[test]
    fn test_two_encryptions_produce_different_ciphertext() {
        let password = b"test-password-16chars!";
        let plaintext = b"secret api key data";

        let protected1 = encrypt_with_password(plaintext, password, SubkeyContext::Practice).unwrap();
        let protected2 = encrypt_with_password(plaintext, password, SubkeyContext::Practice).unwrap();

        assert_ne!(protected1.encrypted.nonce, protected2.encrypted.nonce);
        assert_ne!(protected1.encrypted.ciphertext, protected2.encrypted.ciphertext);
        assert_ne!(protected1.encrypted.salt, protected2.encrypted.salt);
    }

    #[test]
    fn test_hmac_is_constant_time() {
        let password = b"test-password-16chars!";
        let plaintext = b"secret api key data";

        let protected = encrypt_with_password(plaintext, password, SubkeyContext::Practice).unwrap();

        let master_key = derive_master_key(
            password,
            &protected.encrypted.salt,
            &protected.encrypted.kdf_params,
        ).unwrap();
        let hmac_key = derive_subkey(&master_key, SubkeyContext::Hmac).unwrap();
        let blob_json = serde_json::to_vec(&protected.encrypted).unwrap();

        assert!(verify_hmac(&hmac_key, &blob_json, &protected.hmac, protected.encrypted.version));

        let mut bad_hmac = protected.hmac.clone();
        bad_hmac[0] ^= 0xFF;
        assert!(!verify_hmac(&hmac_key, &blob_json, &bad_hmac, protected.encrypted.version));
    }

    // --- AGT-664: HMAC MAC + Argon2 param clamping ---

    /// Rebuild a blob exactly as pre-AGT-664 code did: version 1 with the legacy
    /// hand-rolled secret-prefix MAC `SHA-256(mac_key ‖ blob_json)`.
    fn make_legacy_blob(plaintext: &[u8], password: &[u8], context: SubkeyContext) -> ProtectedBlob {
        let mut protected = encrypt_with_password(plaintext, password, context).unwrap();
        // Stamp the old on-disk version and recompute the MAC over the v1 bytes.
        protected.encrypted.version = 1;

        let master_key = derive_master_key(
            password,
            &protected.encrypted.salt,
            &protected.encrypted.kdf_params,
        )
        .unwrap();
        let hmac_key = derive_subkey(&master_key, SubkeyContext::Hmac).unwrap();
        let blob_json = serde_json::to_vec(&protected.encrypted).unwrap();
        protected.hmac = compute_legacy_mac(&hmac_key, &blob_json).to_vec();
        protected
    }

    #[test]
    fn test_new_blobs_are_written_at_hmac_version() {
        let protected = encrypt_with_password(b"secret", b"test-password-16chars!", SubkeyContext::Practice).unwrap();
        assert_eq!(protected.encrypted.version, HMAC_FORMAT_VERSION);
    }

    #[test]
    fn test_hmac_roundtrip_and_matches_rfc_style_construction() {
        let password = b"test-password-16chars!";
        let plaintext = b"secret api key data";

        let protected = encrypt_with_password(plaintext, password, SubkeyContext::Practice).unwrap();

        let master_key = derive_master_key(
            password,
            &protected.encrypted.salt,
            &protected.encrypted.kdf_params,
        )
        .unwrap();
        let hmac_key = derive_subkey(&master_key, SubkeyContext::Hmac).unwrap();
        let blob_json = serde_json::to_vec(&protected.encrypted).unwrap();

        // compute_hmac must equal a straight HMAC-SHA256 over the same key/data,
        // proving we use the crate primitive and not the old secret-prefix hash.
        let mac_key = derive_mac_key(&hmac_key);
        let mut mac = <HmacSha256 as Mac>::new_from_slice(&mac_key).unwrap();
        mac.update(&blob_json);
        let expected = mac.finalize().into_bytes();

        assert_eq!(&compute_hmac(&hmac_key, &blob_json)[..], &expected[..]);
        // And it must differ from the legacy secret-prefix construction.
        assert_ne!(compute_hmac(&hmac_key, &blob_json), compute_legacy_mac(&hmac_key, &blob_json));

        // Full round-trip still unlocks.
        let decrypted = decrypt_with_password(&protected, password, SubkeyContext::Practice).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_hmac_tamper_detection() {
        let password = b"test-password-16chars!";
        let plaintext = b"secret api key data";

        let protected = encrypt_with_password(plaintext, password, SubkeyContext::Practice).unwrap();

        // Full end-to-end: flipping a MAC byte or a payload byte must fail unlock.
        let mut mac_tampered = protected.clone();
        mac_tampered.hmac[0] ^= 0xFF;
        assert!(decrypt_with_password(&mac_tampered, password, SubkeyContext::Practice).is_err());

        let mut payload_tampered = protected.clone();
        payload_tampered.encrypted.salt[0] ^= 0xFF;
        assert!(decrypt_with_password(&payload_tampered, password, SubkeyContext::Practice).is_err());

        // Exhaustive per-byte sweep at the MAC layer (derive the key once — cheap,
        // no repeated Argon2): every single-byte flip of the MAC is rejected.
        let master_key = derive_master_key(
            password,
            &protected.encrypted.salt,
            &protected.encrypted.kdf_params,
        )
        .unwrap();
        let hmac_key = derive_subkey(&master_key, SubkeyContext::Hmac).unwrap();
        let blob_json = serde_json::to_vec(&protected.encrypted).unwrap();
        let version = protected.encrypted.version;

        assert!(verify_hmac(&hmac_key, &blob_json, &protected.hmac, version));
        for i in 0..protected.hmac.len() {
            let mut bad = protected.hmac.clone();
            bad[i] ^= 0xFF;
            assert!(
                !verify_hmac(&hmac_key, &blob_json, &bad, version),
                "tampered MAC byte {} was accepted",
                i
            );
        }
    }

    #[test]
    fn test_legacy_v1_blob_still_unlocks() {
        let password = b"test-password-16chars!";
        let plaintext = b"legacy secret api key";

        let legacy = make_legacy_blob(plaintext, password, SubkeyContext::Practice);
        assert_eq!(legacy.encrypted.version, 1);

        // Back-compat: a v1 blob written by the old secret-prefix MAC still unlocks.
        let decrypted = decrypt_with_password(&legacy, password, SubkeyContext::Practice).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_legacy_v1_blob_rejects_tampering() {
        let password = b"test-password-16chars!";
        let plaintext = b"legacy secret api key";

        let mut legacy = make_legacy_blob(plaintext, password, SubkeyContext::Practice);
        legacy.hmac[0] ^= 0xFF;
        assert!(decrypt_with_password(&legacy, password, SubkeyContext::Practice).is_err());
    }

    #[test]
    fn test_v2_blob_does_not_accept_legacy_mac_downgrade() {
        let password = b"test-password-16chars!";
        let plaintext = b"secret api key data";

        // Build a v2 blob but attach a legacy-style MAC. A v2 blob must NOT accept it.
        let mut protected = encrypt_with_password(plaintext, password, SubkeyContext::Practice).unwrap();
        assert_eq!(protected.encrypted.version, 2);

        let master_key = derive_master_key(
            password,
            &protected.encrypted.salt,
            &protected.encrypted.kdf_params,
        )
        .unwrap();
        let hmac_key = derive_subkey(&master_key, SubkeyContext::Hmac).unwrap();
        let blob_json = serde_json::to_vec(&protected.encrypted).unwrap();
        protected.hmac = compute_legacy_mac(&hmac_key, &blob_json).to_vec();

        assert!(
            decrypt_with_password(&protected, password, SubkeyContext::Practice).is_err(),
            "v2 blob accepted a downgraded legacy MAC"
        );
    }

    #[test]
    fn test_oversized_memory_kib_rejected_before_allocation() {
        let password = b"test-password-16chars!";
        let plaintext = b"secret api key data";

        let mut protected = encrypt_with_password(plaintext, password, SubkeyContext::Practice).unwrap();
        // A hostile blob requesting a huge Argon2 memory cost must be rejected
        // before any allocation happens.
        protected.encrypted.kdf_params.memory_kib = u32::MAX;

        let err = decrypt_with_password(&protected, password, SubkeyContext::Practice).unwrap_err();
        assert!(err.to_string().contains("memory_kib"), "unexpected error: {}", err);
    }

    #[test]
    fn test_out_of_bounds_iterations_and_parallelism_rejected() {
        let password = b"test-password-16chars!";
        let plaintext = b"secret api key data";

        let base = encrypt_with_password(plaintext, password, SubkeyContext::Practice).unwrap();

        let mut too_many_iters = base.clone();
        too_many_iters.encrypted.kdf_params.iterations = u32::MAX;
        assert!(decrypt_with_password(&too_many_iters, password, SubkeyContext::Practice).is_err());

        let mut zero_parallelism = base.clone();
        zero_parallelism.encrypted.kdf_params.parallelism = 0;
        assert!(decrypt_with_password(&zero_parallelism, password, SubkeyContext::Practice).is_err());
    }

    #[test]
    fn test_default_kdf_params_pass_validation() {
        // The parameters a real vault is written with must never be rejected.
        assert!(validate_kdf_params(&KdfParams::default()).is_ok());
    }

    #[test]
    fn test_generate_nonce_unique() {
        let nonce1 = generate_nonce();
        let nonce2 = generate_nonce();
        assert_ne!(nonce1, nonce2);
    }

    #[test]
    fn test_protected_blob_serialization() {
        let password = b"test-password-16chars!";
        let plaintext = b"secret api key data";

        let protected = encrypt_with_password(plaintext, password, SubkeyContext::Practice).unwrap();
        let encoded = protected.to_base64().unwrap();
        let decoded = ProtectedBlob::from_base64(&encoded).unwrap();

        let decrypted = decrypt_with_password(&decoded, password, SubkeyContext::Practice).unwrap();
        assert_eq!(decrypted, plaintext);
    }
}
