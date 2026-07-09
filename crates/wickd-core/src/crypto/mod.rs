pub mod types;
pub mod kdf;
pub mod encryption;
pub mod vault;
pub mod rate_limit;
pub mod password_strength;
pub mod device;

pub use types::{EncryptedBlob, ProtectedBlob, KdfParams, SubkeyContext};
pub use kdf::{derive_master_key, derive_subkey};
pub use encryption::{encrypt, decrypt, compute_hmac, verify_hmac};
pub use vault::{CredentialVault, OandaCredentials};
pub use rate_limit::{AttemptTracker, RateLimiter};
pub use password_strength::{PasswordStrength, check_password, check_password_local};
pub use device::{DeviceManager, get_default_app_data_dir};
