use sha1::{Sha1, Digest};
use crate::error::{Error, Result};

/// Minimum password length for security
pub const MIN_PASSWORD_LENGTH: usize = 16;

/// Minimum zxcvbn score required (0-4 scale)
pub const MIN_ZXCVBN_SCORE: u8 = 3;

/// Result of password strength analysis
#[derive(Debug, Clone)]
pub struct PasswordStrength {
    /// zxcvbn score (0-4)
    pub score: u8,
    /// Human-readable feedback messages
    pub feedback: Vec<String>,
    /// Whether the password appears in breach databases
    pub is_compromised: bool,
    /// Number of times found in breaches (0 if not compromised)
    pub breach_count: u64,
    /// Whether the password meets all requirements
    pub meets_requirements: bool,
}

/// Check password strength using local validation only (no network calls)
pub fn check_password_local(password: &str) -> PasswordStrength {
    let mut feedback = Vec::new();
    let mut meets_requirements = true;

    // Length check
    if password.len() < MIN_PASSWORD_LENGTH {
        feedback.push(format!(
            "Password must be at least {} characters (currently {})",
            MIN_PASSWORD_LENGTH,
            password.len()
        ));
        meets_requirements = false;
    }

    // Character diversity checks
    let has_uppercase = password.chars().any(|c| c.is_uppercase());
    let has_lowercase = password.chars().any(|c| c.is_lowercase());
    let has_digit = password.chars().any(|c| c.is_ascii_digit());
    let has_special = password.chars().any(|c| !c.is_alphanumeric());

    if !has_uppercase {
        feedback.push("Add uppercase letters for more security".to_string());
    }
    if !has_lowercase {
        feedback.push("Add lowercase letters for more security".to_string());
    }
    if !has_digit {
        feedback.push("Add numbers for more security".to_string());
    }
    if !has_special {
        feedback.push("Add special characters (!@#$%^&*) for more security".to_string());
    }

    // Require all character types
    if !has_uppercase || !has_lowercase || !has_digit || !has_special {
        meets_requirements = false;
    }

    // zxcvbn analysis
    let zxcvbn_result = zxcvbn::zxcvbn(password, &[]);
    let score = zxcvbn_result.score().into();

    if score < MIN_ZXCVBN_SCORE {
        meets_requirements = false;
        feedback.push(format!(
            "Password strength score: {}/4 (minimum {} required)",
            score, MIN_ZXCVBN_SCORE
        ));

        // Add zxcvbn feedback if available
        if let Some(warning) = zxcvbn_result.feedback().as_ref().and_then(|f| f.warning()) {
            feedback.push(format!("Warning: {}", warning));
        }
        if let Some(suggestions) = zxcvbn_result.feedback().as_ref() {
            for suggestion in suggestions.suggestions() {
                feedback.push(format!("Suggestion: {}", suggestion));
            }
        }
    }

    PasswordStrength {
        score,
        feedback,
        is_compromised: false,
        breach_count: 0,
        meets_requirements,
    }
}

/// Check password against HaveIBeenPwned using k-anonymity API
/// This sends only the first 5 characters of the SHA-1 hash to the API
pub async fn check_hibp(password: &str) -> Result<(bool, u64)> {
    // SHA-1 hash of the password
    let mut hasher = Sha1::new();
    hasher.update(password.as_bytes());
    let hash = hasher.finalize();
    let hash_hex = format!("{:X}", hash);

    // Split into prefix (first 5 chars) and suffix (rest)
    let prefix = &hash_hex[..5];
    let suffix = &hash_hex[5..];

    // Query HIBP API with prefix only (k-anonymity)
    let url = format!("https://api.pwnedpasswords.com/range/{}", prefix);

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header("Add-Padding", "true") // Adds padding to prevent timing attacks
        .send()
        .await
        .map_err(|e| Error::Crypto(format!("Failed to check HIBP: {}", e)))?;

    if !response.status().is_success() {
        return Err(Error::Crypto(format!(
            "HIBP API returned status: {}",
            response.status()
        )));
    }

    let body = response
        .text()
        .await
        .map_err(|e| Error::Crypto(format!("Failed to read HIBP response: {}", e)))?;

    // Parse response - each line is "SUFFIX:COUNT"
    for line in body.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 2 {
            let found_suffix = parts[0].trim();
            if found_suffix.eq_ignore_ascii_case(suffix) {
                let count: u64 = parts[1].trim().parse().unwrap_or(1);
                return Ok((true, count));
            }
        }
    }

    Ok((false, 0))
}

/// Full password strength check including HIBP (async)
pub async fn check_password(password: &str) -> Result<PasswordStrength> {
    let mut strength = check_password_local(password);

    // Check HIBP
    match check_hibp(password).await {
        Ok((is_compromised, breach_count)) => {
            strength.is_compromised = is_compromised;
            strength.breach_count = breach_count;

            if is_compromised {
                strength.meets_requirements = false;
                strength.feedback.insert(
                    0,
                    format!(
                        "This password has been found in {} data breach{}. Choose a different password.",
                        breach_count,
                        if breach_count == 1 { "" } else { "es" }
                    ),
                );
            }
        }
        Err(e) => {
            // Log error but don't fail - HIBP check is optional
            tracing::warn!("Failed to check HIBP: {}", e);
            strength.feedback.push(
                "Could not verify password against breach database. Proceed with caution.".to_string(),
            );
        }
    }

    Ok(strength)
}

/// Generate a hash prefix for HIBP lookup (for testing)
pub fn get_hibp_prefix(password: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(password.as_bytes());
    let hash = hasher.finalize();
    let hash_hex = format!("{:X}", hash);
    hash_hex[..5].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_password_fails() {
        let result = check_password_local("Short1!");
        assert!(!result.meets_requirements);
        assert!(result.feedback.iter().any(|f| f.contains("16 characters")));
    }

    #[test]
    fn test_missing_uppercase_feedback() {
        let result = check_password_local("verylongpassword123!");
        assert!(result.feedback.iter().any(|f| f.contains("uppercase")));
    }

    #[test]
    fn test_missing_lowercase_feedback() {
        let result = check_password_local("VERYLONGPASSWORD123!");
        assert!(result.feedback.iter().any(|f| f.contains("lowercase")));
    }

    #[test]
    fn test_missing_digit_feedback() {
        let result = check_password_local("VeryLongPassword!!!!");
        assert!(result.feedback.iter().any(|f| f.contains("numbers")));
    }

    #[test]
    fn test_missing_special_feedback() {
        let result = check_password_local("VeryLongPassword1234");
        assert!(result.feedback.iter().any(|f| f.contains("special")));
    }

    #[test]
    fn test_strong_password_passes() {
        let result = check_password_local("MyStr0ng&Secure#Pass!");
        // Should meet all character requirements
        assert!(
            !result.feedback.iter().any(|f| f.contains("uppercase")),
            "Should not need uppercase"
        );
        assert!(
            !result.feedback.iter().any(|f| f.contains("lowercase")),
            "Should not need lowercase"
        );
        assert!(
            !result.feedback.iter().any(|f| f.contains("numbers")),
            "Should not need numbers"
        );
        assert!(
            !result.feedback.iter().any(|f| f.contains("special")),
            "Should not need special chars"
        );
    }

    #[test]
    fn test_zxcvbn_score_range() {
        // Simple password should have low score
        let weak = check_password_local("password12345678");
        assert!(weak.score <= 2);

        // Complex password should have higher score
        let strong = check_password_local("xK9#mP$2qL@8nR!4");
        assert!(strong.score >= 3);
    }

    #[test]
    fn test_common_password_fails_zxcvbn() {
        let result = check_password_local("Password123456!!");
        // Common patterns should get flagged by zxcvbn
        assert!(result.score < 4);
    }

    #[test]
    fn test_hibp_prefix_format() {
        let prefix = get_hibp_prefix("password");
        assert_eq!(prefix.len(), 5);
        assert!(prefix.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_hibp_prefix_deterministic() {
        let prefix1 = get_hibp_prefix("testpassword");
        let prefix2 = get_hibp_prefix("testpassword");
        assert_eq!(prefix1, prefix2);
    }

    #[test]
    fn test_different_passwords_different_prefixes() {
        let prefix1 = get_hibp_prefix("password1");
        let prefix2 = get_hibp_prefix("password2");
        // Different passwords should likely have different prefixes
        // (not guaranteed but highly probable)
        assert_ne!(prefix1, prefix2);
    }

    #[test]
    fn test_password_strength_struct() {
        let strength = PasswordStrength {
            score: 3,
            feedback: vec!["Test feedback".to_string()],
            is_compromised: false,
            breach_count: 0,
            meets_requirements: true,
        };

        assert_eq!(strength.score, 3);
        assert!(!strength.is_compromised);
        assert!(strength.meets_requirements);
    }

    #[tokio::test]
    async fn test_check_hibp_known_compromised() {
        // "password" is definitely in HIBP
        let result = check_hibp("password").await;
        assert!(result.is_ok());
        let (is_compromised, count) = result.unwrap();
        assert!(is_compromised, "Common password 'password' should be compromised");
        assert!(count > 0, "Should have breach count > 0");
    }

    #[tokio::test]
    async fn test_check_hibp_random_password() {
        // A random UUID-like password should not be in HIBP
        let result = check_hibp("xK9mP2qL8nR4vT6w").await;
        assert!(result.is_ok());
        let (is_compromised, _) = result.unwrap();
        // Note: This might theoretically be compromised, but highly unlikely
        assert!(!is_compromised, "Random password should not be compromised");
    }
}
