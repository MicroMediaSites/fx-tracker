use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// On-disk blob format version.
///
/// - v1: pre-AGT-664 hand-rolled secret-prefix MAC `SHA-256(mac_key ‖ data)`.
///       Accepted on read for back-compat only (see [`crate::crypto::encryption`]).
/// - v2: `HMAC-SHA256(mac_key, data)`. All new blobs are written at this version;
///       reading a v1 blob and re-saving it transparently migrates it to v2.
pub const BLOB_VERSION: u8 = 2;
/// First blob version whose MAC is a proper HMAC. Verification of a blob at this
/// version or later NEVER falls back to the legacy MAC (downgrade protection).
pub const HMAC_FORMAT_VERSION: u8 = 2;
pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 12;
pub const TAG_LEN: usize = 16;
pub const KEY_LEN: usize = 32;
pub const HMAC_LEN: usize = 32;

pub const DEFAULT_KDF_MEMORY: u32 = 128 * 1024;
pub const DEFAULT_KDF_ITERATIONS: u32 = 4;
pub const DEFAULT_KDF_PARALLELISM: u32 = 4;

// Sane bounds for KDF parameters read from an untrusted blob. Parameters are
// validated against these BEFORE any Argon2 memory allocation, so a hostile blob
// cannot trigger a memory-exhaustion DoS on unlock. The defaults above sit
// comfortably inside these bounds, so legitimate vaults are never rejected.
pub const MIN_KDF_MEMORY: u32 = 8 * 1024; // 8 MiB floor
pub const MAX_KDF_MEMORY: u32 = 1024 * 1024; // 1 GiB ceiling
pub const MIN_KDF_ITERATIONS: u32 = 1;
pub const MAX_KDF_ITERATIONS: u32 = 16;
pub const MIN_KDF_PARALLELISM: u32 = 1;
pub const MAX_KDF_PARALLELISM: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            memory_kib: DEFAULT_KDF_MEMORY,
            iterations: DEFAULT_KDF_ITERATIONS,
            parallelism: DEFAULT_KDF_PARALLELISM,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubkeyContext {
    Practice,
    Live,
    Hmac,
    Trading,
}

impl SubkeyContext {
    pub fn as_bytes(&self) -> &'static [u8] {
        match self {
            SubkeyContext::Practice => b"fx-tracker-practice-v1",
            SubkeyContext::Live => b"fx-tracker-live-v1",
            SubkeyContext::Hmac => b"fx-tracker-hmac-v1",
            SubkeyContext::Trading => b"fx-tracker-trading-v1",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedBlob {
    pub version: u8,
    #[serde(with = "base64_bytes")]
    pub salt: Vec<u8>,
    pub kdf_params: KdfParams,
    #[serde(with = "base64_bytes")]
    pub nonce: Vec<u8>,
    #[serde(with = "base64_bytes")]
    pub ciphertext: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtectedBlob {
    pub encrypted: EncryptedBlob,
    #[serde(with = "base64_bytes")]
    pub hmac: Vec<u8>,
}

impl ProtectedBlob {
    pub fn to_base64(&self) -> Result<String, serde_json::Error> {
        let json = serde_json::to_vec(self)?;
        Ok(base64_encode(&json))
    }

    pub fn from_base64(encoded: &str) -> Result<Self, CryptoParseError> {
        let decoded = base64_decode(encoded).map_err(|_| CryptoParseError::InvalidBase64)?;
        serde_json::from_slice(&decoded).map_err(|_| CryptoParseError::InvalidJson)
    }
}

#[derive(Debug, Clone, Zeroize, ZeroizeOnDrop)]
pub struct MasterKey {
    key: [u8; KEY_LEN],
}

impl MasterKey {
    pub fn new(key: [u8; KEY_LEN]) -> Self {
        Self { key }
    }

    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.key
    }
}

#[derive(Debug, Clone, Zeroize, ZeroizeOnDrop)]
pub struct DerivedKey {
    key: [u8; KEY_LEN],
}

impl DerivedKey {
    pub fn new(key: [u8; KEY_LEN]) -> Self {
        Self { key }
    }

    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.key
    }
}

#[derive(Debug, Clone)]
pub enum CryptoParseError {
    InvalidBase64,
    InvalidJson,
}

impl std::fmt::Display for CryptoParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptoParseError::InvalidBase64 => write!(f, "Invalid base64 encoding"),
            CryptoParseError::InvalidJson => write!(f, "Invalid JSON structure"),
        }
    }
}

impl std::error::Error for CryptoParseError {}

mod base64_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&super::base64_encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        super::base64_decode(&s).map_err(serde::de::Error::custom)
    }
}

fn base64_encode(data: &[u8]) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine};
    STANDARD.encode(data)
}

fn base64_decode(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    STANDARD.decode(s)
}
