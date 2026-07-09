use crate::error::{Error, Result};

/// Build mode determines AI model tiers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildMode {
    Dev,      // Pro → Haiku, Advanced → Sonnet
    Staging,  // Pro → Sonnet, Advanced → Opus
    Prod,     // Pro → Sonnet, Advanced → Opus
}

impl BuildMode {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "staging" => BuildMode::Staging,
            "prod" | "production" => BuildMode::Prod,
            _ => BuildMode::Dev, // Default to dev for safety
        }
    }

    /// Returns true if this build should use production AI models (Sonnet/Opus)
    pub fn use_production_models(&self) -> bool {
        matches!(self, BuildMode::Staging | BuildMode::Prod)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OandaEnvironment {
    Practice,
    Live,
}

impl OandaEnvironment {
    pub fn api_base_url(&self) -> &'static str {
        match self {
            OandaEnvironment::Practice => "https://api-fxpractice.oanda.com",
            OandaEnvironment::Live => "https://api-fxtrade.oanda.com",
        }
    }

    pub fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "practice" | "demo" => Ok(OandaEnvironment::Practice),
            "live" => Ok(OandaEnvironment::Live),
            other => Err(Error::Config(format!(
                "Invalid OANDA_ENVIRONMENT '{}'. Expected 'practice' or 'live'",
                other
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    /// OANDA API key - optional at startup, set after vault unlock
    pub api_key: Option<String>,
    /// OANDA account ID - optional at startup, set after vault unlock
    pub account_id: Option<String>,
    pub environment: OandaEnvironment,
    pub anthropic_api_key: Option<String>,
    /// Build mode (dev/staging/prod) - determines AI model tiers
    pub build_mode: BuildMode,
}

/// Compile-time configuration values that can be baked into the binary via option_env!()
/// These serve as fallbacks when runtime .env values are not found.
///
/// SECURITY: the Anthropic API key is deliberately NOT part of this struct. It
/// must only ever be resolved at runtime (encrypted vault or process env) so a
/// `strings` on the shipped binary can never yield a key. See AGT-657.
#[derive(Default)]
pub struct CompileTimeConfig {
    pub build_mode: Option<&'static str>,
}

impl Config {
    /// Load config with compile-time fallbacks for CI builds.
    /// Priority: runtime .env > compile-time option_env!()
    pub fn load_with_compile_time(compile_time: CompileTimeConfig) -> Result<Self> {
        // Try loading .env from multiple locations:
        // 1. Current directory (dev mode)
        // 2. src-tauri/.env (for Rust-specific vars when running from project root)
        // 3. Application Support directory (installed app)
        dotenvy::dotenv().ok();

        // Also try src-tauri/.env for when running from project root
        dotenvy::from_path("src-tauri/.env").ok();

        // Also try Application Support for installed apps
        if let Some(home) = dirs::home_dir() {
            // Try new app identifier first
            let app_support_env = home
                .join("Library/Application Support/com.candlesight.app/.env");
            if app_support_env.exists() {
                dotenvy::from_path(&app_support_env).ok();
            } else {
                // Fall back to old identifier for backwards compatibility
                let legacy_env = home
                    .join("Library/Application Support/com.fx-tracker.app/.env");
                if legacy_env.exists() {
                    dotenvy::from_path(&legacy_env).ok();
                }
            }
        }

        // OANDA credentials are optional - they come from the vault now
        let api_key = std::env::var("OANDA_API_KEY")
            .ok()
            .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
            .filter(|s| !s.is_empty());

        let account_id = std::env::var("OANDA_ACCOUNT_ID")
            .ok()
            .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
            .filter(|s| !s.is_empty());

        let environment_str = std::env::var("OANDA_ENVIRONMENT")
            .unwrap_or_else(|_| "practice".to_string());

        let environment = OandaEnvironment::from_str(&environment_str)?;

        // Anthropic API key: resolved SOLELY at runtime from the encrypted
        // vault or process env — never baked into the binary at compile time
        // (a `strings` on the shipped .app must never yield a key). See AGT-657.
        // Trim whitespace and strip quotes that might be included in .env file.
        let anthropic_api_key = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .map(|key| {
                let trimmed = key.trim();
                // Strip surrounding quotes if present (some .env parsers leave them)
                let stripped = trimmed
                    .strip_prefix('"').unwrap_or(trimmed)
                    .strip_suffix('"').unwrap_or(trimmed)
                    .strip_prefix('\'').unwrap_or(trimmed)
                    .strip_suffix('\'').unwrap_or(trimmed);
                stripped.to_string()
            })
            .filter(|key| !key.is_empty());

        // Build mode: runtime .env > compile-time > dev default
        let build_mode = std::env::var("BUILD_MODE")
            .ok()
            .or_else(|| compile_time.build_mode.map(String::from))
            .map(|s| BuildMode::from_str(&s))
            .unwrap_or(BuildMode::Dev);

        Ok(Config {
            api_key,
            account_id,
            environment,
            anthropic_api_key,
            build_mode,
        })
    }

    /// Load config from runtime .env only (backwards compatibility)
    pub fn load() -> Result<Self> {
        Self::load_with_compile_time(CompileTimeConfig::default())
    }

    /// Check if OANDA credentials are configured
    pub fn has_oanda_credentials(&self) -> bool {
        self.api_key.is_some() && self.account_id.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_environment_parsing_practice() {
        assert_eq!(
            OandaEnvironment::from_str("practice").unwrap(),
            OandaEnvironment::Practice
        );
        assert_eq!(
            OandaEnvironment::from_str("PRACTICE").unwrap(),
            OandaEnvironment::Practice
        );
        assert_eq!(
            OandaEnvironment::from_str("Practice").unwrap(),
            OandaEnvironment::Practice
        );
    }

    #[test]
    fn test_environment_parsing_demo_alias() {
        assert_eq!(
            OandaEnvironment::from_str("demo").unwrap(),
            OandaEnvironment::Practice
        );
        assert_eq!(
            OandaEnvironment::from_str("DEMO").unwrap(),
            OandaEnvironment::Practice
        );
    }

    #[test]
    fn test_environment_parsing_live() {
        assert_eq!(
            OandaEnvironment::from_str("live").unwrap(),
            OandaEnvironment::Live
        );
        assert_eq!(
            OandaEnvironment::from_str("LIVE").unwrap(),
            OandaEnvironment::Live
        );
        assert_eq!(
            OandaEnvironment::from_str("Live").unwrap(),
            OandaEnvironment::Live
        );
    }

    #[test]
    fn test_environment_parsing_invalid() {
        let result = OandaEnvironment::from_str("invalid");
        assert!(result.is_err());

        let err = result.unwrap_err();
        match err {
            Error::Config(msg) => {
                assert!(msg.contains("invalid"));
                assert!(msg.contains("Invalid OANDA_ENVIRONMENT"));
            }
            _ => panic!("Expected Config error"),
        }
    }

    #[test]
    fn test_environment_parsing_empty_string() {
        let result = OandaEnvironment::from_str("");
        assert!(result.is_err());
    }

    #[test]
    fn test_api_base_urls() {
        assert_eq!(
            OandaEnvironment::Practice.api_base_url(),
            "https://api-fxpractice.oanda.com"
        );
        assert_eq!(
            OandaEnvironment::Live.api_base_url(),
            "https://api-fxtrade.oanda.com"
        );
    }

    #[test]
    fn test_practice_and_live_have_different_urls() {
        let practice_url = OandaEnvironment::Practice.api_base_url();
        let live_url = OandaEnvironment::Live.api_base_url();
        assert_ne!(practice_url, live_url);
        assert!(practice_url.contains("practice"));
        assert!(live_url.contains("fxtrade"));
    }

    #[test]
    fn test_environment_copy_and_clone() {
        let env = OandaEnvironment::Practice;
        let cloned = env.clone();
        let copied = env;

        assert_eq!(env, cloned);
        assert_eq!(env, copied);
        assert_eq!(cloned, copied);
    }

    #[test]
    fn test_environment_debug() {
        let practice = OandaEnvironment::Practice;
        let live = OandaEnvironment::Live;

        let practice_debug = format!("{:?}", practice);
        let live_debug = format!("{:?}", live);

        assert!(practice_debug.contains("Practice"));
        assert!(live_debug.contains("Live"));
    }

    #[test]
    fn test_api_urls_are_https() {
        assert!(OandaEnvironment::Practice.api_base_url().starts_with("https://"));
        assert!(OandaEnvironment::Live.api_base_url().starts_with("https://"));
    }

    #[test]
    fn test_api_urls_are_oanda_domain() {
        assert!(OandaEnvironment::Practice.api_base_url().contains("oanda.com"));
        assert!(OandaEnvironment::Live.api_base_url().contains("oanda.com"));
    }

    // AGT-657: the Anthropic key must resolve SOLELY at runtime — there is no
    // compile-time bake fallback. `CompileTimeConfig` carries only `build_mode`.
    #[test]
    fn test_anthropic_key_resolves_from_runtime_env() {
        // A runtime env value flows through to Config.
        std::env::set_var("ANTHROPIC_API_KEY", "test-runtime-anthropic-key");
        let config = Config::load().expect("load config");
        assert_eq!(
            config.anthropic_api_key.as_deref(),
            Some("test-runtime-anthropic-key"),
            "runtime ANTHROPIC_API_KEY must resolve into Config",
        );

        // An empty runtime value resolves to None — no baked fallback exists.
        std::env::set_var("ANTHROPIC_API_KEY", "");
        let config = Config::load().expect("load config");
        assert_eq!(
            config.anthropic_api_key, None,
            "empty ANTHROPIC_API_KEY must resolve to None with no fallback",
        );

        std::env::remove_var("ANTHROPIC_API_KEY");
    }
}
