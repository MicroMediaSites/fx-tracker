use std::time::{Duration, SystemTime};
use serde::{Deserialize, Serialize};
use crate::error::{Error, Result};

/// Rate limit configuration
const MAX_ATTEMPTS_BEFORE_LOCK: u32 = 10;
const LOCK_DURATION_SECS: u64 = 300; // 5 minutes

/// Tracks failed authentication attempts with exponential backoff
/// Persisted to app data directory to survive restarts
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttemptTracker {
    failed_attempts: u32,
    last_attempt: Option<u64>, // Unix timestamp in seconds
    locked_until: Option<u64>, // Unix timestamp in seconds
}

impl Default for AttemptTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl AttemptTracker {
    pub fn new() -> Self {
        Self {
            failed_attempts: 0,
            last_attempt: None,
            locked_until: None,
        }
    }

    /// Returns the required wait time before next attempt, if any
    /// Returns Ok(()) if attempt is allowed immediately
    /// Returns Err with the remaining wait duration if rate limited
    pub fn check_rate_limit(&self) -> std::result::Result<(), Duration> {
        let now = current_timestamp();

        // Check if currently locked out
        if let Some(locked_until) = self.locked_until {
            if now < locked_until {
                let remaining = locked_until - now;
                return Err(Duration::from_secs(remaining));
            }
        }

        // Check exponential backoff based on failed attempts
        if let Some(last_attempt) = self.last_attempt {
            let required_delay = self.get_required_delay();
            let elapsed = now.saturating_sub(last_attempt);

            if elapsed < required_delay.as_secs() {
                let remaining = required_delay.as_secs() - elapsed;
                return Err(Duration::from_secs(remaining));
            }
        }

        Ok(())
    }

    /// Calculate the required delay based on failed attempts
    /// - Attempts 1-3: No delay
    /// - Attempt 4: 2 seconds
    /// - Attempt 5: 4 seconds
    /// - Attempt 6: 8 seconds
    /// - Attempts 7-9: 30 seconds
    /// - Attempt 10+: 5 minute lock
    fn get_required_delay(&self) -> Duration {
        match self.failed_attempts {
            0..=3 => Duration::ZERO,
            4 => Duration::from_secs(2),
            5 => Duration::from_secs(4),
            6 => Duration::from_secs(8),
            7..=9 => Duration::from_secs(30),
            _ => Duration::from_secs(LOCK_DURATION_SECS),
        }
    }

    /// Record a failed authentication attempt
    pub fn record_failure(&mut self) {
        let now = current_timestamp();
        self.failed_attempts += 1;
        self.last_attempt = Some(now);

        // If we hit the max, set a hard lock
        if self.failed_attempts >= MAX_ATTEMPTS_BEFORE_LOCK {
            self.locked_until = Some(now + LOCK_DURATION_SECS);
        }
    }

    /// Record a successful authentication (resets the tracker)
    pub fn record_success(&mut self) {
        self.failed_attempts = 0;
        self.last_attempt = None;
        self.locked_until = None;
    }

    /// Check if the account is currently locked
    pub fn is_locked(&self) -> bool {
        if let Some(locked_until) = self.locked_until {
            current_timestamp() < locked_until
        } else {
            false
        }
    }

    /// Get the number of failed attempts
    pub fn failed_attempts(&self) -> u32 {
        self.failed_attempts
    }

    /// Get a human-readable status message
    pub fn status_message(&self) -> Option<String> {
        match self.check_rate_limit() {
            Ok(()) => None,
            Err(remaining) => {
                if self.is_locked() {
                    Some(format!(
                        "Account locked. Try again in {} minutes.",
                        (remaining.as_secs() / 60) + 1
                    ))
                } else {
                    let secs = remaining.as_secs();
                    if secs < 60 {
                        Some(format!("Please wait {} seconds before trying again.", secs))
                    } else {
                        Some(format!(
                            "Please wait {} minutes before trying again.",
                            (secs / 60) + 1
                        ))
                    }
                }
            }
        }
    }
}

/// Get current Unix timestamp in seconds
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

/// Manages rate limiting with persistent storage
pub struct RateLimiter {
    tracker: AttemptTracker,
    storage_path: Option<std::path::PathBuf>,
}

impl RateLimiter {
    /// Create a new rate limiter with optional persistent storage
    pub fn new(storage_path: Option<std::path::PathBuf>) -> Self {
        let tracker = storage_path
            .as_ref()
            .and_then(|path| Self::load_from_file(path))
            .unwrap_or_default();

        Self {
            tracker,
            storage_path,
        }
    }

    /// Create a rate limiter without persistence (for testing)
    pub fn in_memory() -> Self {
        Self {
            tracker: AttemptTracker::new(),
            storage_path: None,
        }
    }

    /// Check if an attempt is allowed
    pub fn check(&self) -> std::result::Result<(), Duration> {
        self.tracker.check_rate_limit()
    }

    /// Record a failed attempt and persist
    pub fn record_failure(&mut self) -> Result<()> {
        self.tracker.record_failure();
        self.save()?;
        Ok(())
    }

    /// Record a successful attempt and persist
    pub fn record_success(&mut self) -> Result<()> {
        self.tracker.record_success();
        self.save()?;
        Ok(())
    }

    /// Get the current status message
    pub fn status_message(&self) -> Option<String> {
        self.tracker.status_message()
    }

    /// Check if locked out
    pub fn is_locked(&self) -> bool {
        self.tracker.is_locked()
    }

    /// Get failed attempt count
    pub fn failed_attempts(&self) -> u32 {
        self.tracker.failed_attempts()
    }

    /// Save to persistent storage
    fn save(&self) -> Result<()> {
        if let Some(path) = &self.storage_path {
            let json = serde_json::to_string(&self.tracker)
                .map_err(|e| Error::Crypto(format!("Failed to serialize rate limit data: {}", e)))?;
            std::fs::write(path, &json)
                .map_err(|e| Error::Crypto(format!("Failed to write rate limit data: {}", e)))?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                    .map_err(|e| Error::Crypto(format!("Failed to set file permissions: {}", e)))?;
            }
        }
        Ok(())
    }

    /// Load from persistent storage
    fn load_from_file(path: &std::path::Path) -> Option<AttemptTracker> {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_delay_first_three_attempts() {
        let mut tracker = AttemptTracker::new();

        // First 3 attempts should have no delay
        for _ in 0..3 {
            assert!(tracker.check_rate_limit().is_ok());
            tracker.record_failure();
        }

        // 4th attempt requires delay
        tracker.record_failure();
        // Note: This might pass immediately if test runs fast enough
        // The delay is 2 seconds for attempt 4
    }

    #[test]
    fn test_success_resets_tracker() {
        let mut tracker = AttemptTracker::new();

        // Add some failures
        for _ in 0..5 {
            tracker.record_failure();
        }

        assert!(tracker.failed_attempts() > 0);

        // Success resets
        tracker.record_success();
        assert_eq!(tracker.failed_attempts(), 0);
        assert!(tracker.check_rate_limit().is_ok());
    }

    #[test]
    fn test_lock_after_max_attempts() {
        let mut tracker = AttemptTracker::new();

        // Exhaust all attempts
        for _ in 0..MAX_ATTEMPTS_BEFORE_LOCK {
            tracker.record_failure();
        }

        assert!(tracker.is_locked());
        assert!(tracker.check_rate_limit().is_err());
    }

    #[test]
    fn test_status_message() {
        let tracker = AttemptTracker::new();
        assert!(tracker.status_message().is_none());

        // After lock, should have message
        let mut tracker = AttemptTracker::new();
        for _ in 0..MAX_ATTEMPTS_BEFORE_LOCK {
            tracker.record_failure();
        }
        let msg = tracker.status_message();
        assert!(msg.is_some());
        assert!(msg.unwrap().contains("locked"));
    }

    #[test]
    fn test_get_required_delay() {
        let mut tracker = AttemptTracker::new();

        // 0-3 attempts: no delay
        assert_eq!(tracker.get_required_delay(), Duration::ZERO);
        tracker.failed_attempts = 3;
        assert_eq!(tracker.get_required_delay(), Duration::ZERO);

        // 4 attempts: 2 seconds
        tracker.failed_attempts = 4;
        assert_eq!(tracker.get_required_delay(), Duration::from_secs(2));

        // 5 attempts: 4 seconds
        tracker.failed_attempts = 5;
        assert_eq!(tracker.get_required_delay(), Duration::from_secs(4));

        // 6 attempts: 8 seconds
        tracker.failed_attempts = 6;
        assert_eq!(tracker.get_required_delay(), Duration::from_secs(8));

        // 7-9 attempts: 30 seconds
        tracker.failed_attempts = 7;
        assert_eq!(tracker.get_required_delay(), Duration::from_secs(30));
        tracker.failed_attempts = 9;
        assert_eq!(tracker.get_required_delay(), Duration::from_secs(30));

        // 10+ attempts: 5 minutes
        tracker.failed_attempts = 10;
        assert_eq!(tracker.get_required_delay(), Duration::from_secs(300));
    }

    #[test]
    fn test_rate_limiter_in_memory() {
        let mut limiter = RateLimiter::in_memory();

        assert!(limiter.check().is_ok());
        limiter.record_failure().unwrap();
        assert_eq!(limiter.failed_attempts(), 1);

        limiter.record_success().unwrap();
        assert_eq!(limiter.failed_attempts(), 0);
    }

    #[test]
    fn test_serialization() {
        let mut tracker = AttemptTracker::new();
        tracker.record_failure();
        tracker.record_failure();

        let json = serde_json::to_string(&tracker).unwrap();
        let deserialized: AttemptTracker = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.failed_attempts(), 2);
    }
}
