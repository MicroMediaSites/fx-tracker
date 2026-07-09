use std::path::PathBuf;
use uuid::Uuid;
use crate::error::{Error, Result};

const DEVICE_ID_FILE: &str = "device_id";

/// Manages unique device identification for per-device credential encryption
///
/// Each device gets a unique UUID that:
/// - Is generated on first launch
/// - Is stored in the Tauri app data directory
/// - Persists across app restarts
/// - Is used to identify credential blobs for this device
pub struct DeviceManager {
    app_data_dir: PathBuf,
}

impl DeviceManager {
    /// Create a new DeviceManager with the specified app data directory
    pub fn new(app_data_dir: PathBuf) -> Self {
        Self { app_data_dir }
    }

    /// Get or create the device ID
    ///
    /// Returns the existing device ID if one exists, or generates and stores
    /// a new UUID if this is the first launch on this device.
    pub fn get_or_create_device_id(&self) -> Result<String> {
        let device_id_path = self.app_data_dir.join(DEVICE_ID_FILE);

        // Try to read existing device ID
        if device_id_path.exists() {
            let id = std::fs::read_to_string(&device_id_path)
                .map_err(|e| Error::Crypto(format!("Failed to read device ID: {}", e)))?;
            let id = id.trim().to_string();

            // Validate it's a valid UUID
            if Uuid::parse_str(&id).is_ok() {
                return Ok(id);
            }
            // If invalid, regenerate
            tracing::warn!("Invalid device ID found, regenerating");
        }

        // Generate new device ID
        let device_id = Uuid::new_v4().to_string();
        self.store_device_id(&device_id)?;
        Ok(device_id)
    }

    /// Store a device ID (internal use, mainly for testing)
    fn store_device_id(&self, device_id: &str) -> Result<()> {
        // Ensure directory exists
        std::fs::create_dir_all(&self.app_data_dir)
            .map_err(|e| Error::Crypto(format!("Failed to create app data directory: {}", e)))?;

        let device_id_path = self.app_data_dir.join(DEVICE_ID_FILE);
        std::fs::write(&device_id_path, device_id)
            .map_err(|e| Error::Crypto(format!("Failed to write device ID: {}", e)))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&device_id_path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| Error::Crypto(format!("Failed to set file permissions: {}", e)))?;
        }

        Ok(())
    }

    /// Check if a device ID exists
    pub fn has_device_id(&self) -> bool {
        let device_id_path = self.app_data_dir.join(DEVICE_ID_FILE);
        device_id_path.exists()
    }

    /// Get the path to the device ID file
    pub fn device_id_path(&self) -> PathBuf {
        self.app_data_dir.join(DEVICE_ID_FILE)
    }

    /// Get the path to the rate limit file
    pub fn rate_limit_path(&self) -> PathBuf {
        self.app_data_dir.join("rate_limit.json")
    }
}

/// Get the default app data directory for wickd
///
/// Uses the `dirs` crate to find the appropriate location:
/// - macOS: ~/Library/Application Support/com.openthink.wickd
/// - Windows: %APPDATA%/wickd
/// - Linux: ~/.local/share/wickd
///
/// If the new directory doesn't exist but the legacy directory does,
/// migrates data from the legacy location and cleans up the old files.
pub fn get_default_app_data_dir() -> Result<PathBuf> {
    let new_dir = get_new_app_data_dir()?;

    // Check if we need to migrate from the legacy location
    if !new_dir.exists() {
        if let Ok(legacy_dir) = get_legacy_app_data_dir() {
            if legacy_dir.exists() {
                migrate_app_data_dir(&legacy_dir, &new_dir);
            }
        }
    }

    Ok(new_dir)
}

/// Get the new (current) app data directory path
fn get_new_app_data_dir() -> Result<PathBuf> {
    let base = dirs::data_dir()
        .ok_or_else(|| Error::Crypto("Could not determine app data directory".to_string()))?;

    #[cfg(target_os = "macos")]
    let app_dir = base.join("com.openthink.wickd");

    #[cfg(target_os = "windows")]
    let app_dir = base.join("wickd");

    #[cfg(target_os = "linux")]
    let app_dir = base.join("wickd");

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    let app_dir = base.join("wickd");

    Ok(app_dir)
}

/// Get the legacy app data directory path (for migration)
fn get_legacy_app_data_dir() -> Result<PathBuf> {
    let base = dirs::data_dir()
        .ok_or_else(|| Error::Crypto("Could not determine app data directory".to_string()))?;

    #[cfg(target_os = "macos")]
    let app_dir = base.join("com.candlesight.app");

    #[cfg(target_os = "windows")]
    let app_dir = base.join("CandleSight");

    #[cfg(target_os = "linux")]
    let app_dir = base.join("candlesight");

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    let app_dir = base.join("candlesight");

    Ok(app_dir)
}

/// Migrate app data from legacy directory to new directory
fn migrate_app_data_dir(old: &std::path::Path, new: &std::path::Path) {
    use tracing::info;

    // Create new directory
    if std::fs::create_dir_all(new).is_err() {
        return;
    }

    // Copy files
    for file in ["device_id", "rate_limit.json"] {
        let old_file = old.join(file);
        if old_file.exists() {
            let _ = std::fs::copy(&old_file, new.join(file));
        }
    }

    // Verify migration succeeded, then cleanup
    if new.join("device_id").exists() || !old.join("device_id").exists() {
        // Delete old files
        let _ = std::fs::remove_file(old.join("device_id"));
        let _ = std::fs::remove_file(old.join("rate_limit.json"));
        // Try to remove old directory (will fail if not empty, which is fine)
        let _ = std::fs::remove_dir(old);
        info!("[Migration] Migrated app data from {:?} to {:?} and cleaned up", old, new);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_temp_dir() -> TempDir {
        tempfile::tempdir().expect("Failed to create temp directory")
    }

    #[test]
    fn test_generates_device_id_on_first_launch() {
        let temp_dir = setup_temp_dir();
        let manager = DeviceManager::new(temp_dir.path().to_path_buf());

        assert!(!manager.has_device_id());

        let device_id = manager.get_or_create_device_id().unwrap();

        // Should be a valid UUID
        assert!(Uuid::parse_str(&device_id).is_ok());
        assert!(manager.has_device_id());
    }

    #[test]
    fn test_returns_same_id_on_subsequent_launches() {
        let temp_dir = setup_temp_dir();
        let manager = DeviceManager::new(temp_dir.path().to_path_buf());

        let id1 = manager.get_or_create_device_id().unwrap();
        let id2 = manager.get_or_create_device_id().unwrap();

        assert_eq!(id1, id2);
    }

    #[test]
    fn test_persists_across_manager_instances() {
        let temp_dir = setup_temp_dir();

        let id1 = {
            let manager = DeviceManager::new(temp_dir.path().to_path_buf());
            manager.get_or_create_device_id().unwrap()
        };

        let id2 = {
            let manager = DeviceManager::new(temp_dir.path().to_path_buf());
            manager.get_or_create_device_id().unwrap()
        };

        assert_eq!(id1, id2);
    }

    #[test]
    fn test_regenerates_invalid_device_id() {
        let temp_dir = setup_temp_dir();
        let manager = DeviceManager::new(temp_dir.path().to_path_buf());

        // Write invalid device ID
        fs::create_dir_all(temp_dir.path()).unwrap();
        fs::write(temp_dir.path().join(DEVICE_ID_FILE), "not-a-uuid").unwrap();

        let device_id = manager.get_or_create_device_id().unwrap();

        // Should have regenerated a valid UUID
        assert!(Uuid::parse_str(&device_id).is_ok());
        assert_ne!(device_id, "not-a-uuid");
    }

    #[test]
    fn test_device_id_file_path() {
        let temp_dir = setup_temp_dir();
        let manager = DeviceManager::new(temp_dir.path().to_path_buf());

        let expected_path = temp_dir.path().join(DEVICE_ID_FILE);
        assert_eq!(manager.device_id_path(), expected_path);
    }

    #[test]
    fn test_rate_limit_file_path() {
        let temp_dir = setup_temp_dir();
        let manager = DeviceManager::new(temp_dir.path().to_path_buf());

        let expected_path = temp_dir.path().join("rate_limit.json");
        assert_eq!(manager.rate_limit_path(), expected_path);
    }

    #[test]
    fn test_creates_directory_if_missing() {
        let temp_dir = setup_temp_dir();
        let nested_path = temp_dir.path().join("nested").join("path");
        let manager = DeviceManager::new(nested_path.clone());

        assert!(!nested_path.exists());

        manager.get_or_create_device_id().unwrap();

        assert!(nested_path.exists());
    }

    #[test]
    fn test_default_app_data_dir() {
        let result = get_default_app_data_dir();
        assert!(result.is_ok());
        let path = result.unwrap();

        // Should be a non-empty path
        assert!(!path.as_os_str().is_empty());

        // Should contain our app identifier
        let path_str = path.to_string_lossy();
        assert!(
            path_str.contains("wickd"),
            "Path should contain app identifier: {}",
            path_str
        );
    }
}
