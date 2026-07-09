use reqwest::{Client, tls};
use crate::config::{Config, OandaEnvironment};
use crate::error::Result;

#[derive(Debug, Clone)]
pub struct OandaClient {
    client: Client,
    base_url: String,
    api_key: String,
    account_id: String,
    environment: OandaEnvironment,
}

impl OandaClient {
    pub fn new(config: &Config) -> Result<Self> {
        let client = Client::builder()
            .min_tls_version(tls::Version::TLS_1_2)
            .build()?;

        Ok(Self {
            client,
            base_url: config.environment.api_base_url().to_string(),
            api_key: config.api_key.clone().unwrap_or_default(),
            account_id: config.account_id.clone().unwrap_or_default(),
            environment: config.environment,
        })
    }

    /// Create a new client for a specific environment, reusing credentials from config
    pub fn for_environment(config: &Config, environment: OandaEnvironment) -> Result<Self> {
        let client = Client::builder()
            .min_tls_version(tls::Version::TLS_1_2)
            .build()?;

        Ok(Self {
            client,
            base_url: environment.api_base_url().to_string(),
            api_key: config.api_key.clone().unwrap_or_default(),
            account_id: config.account_id.clone().unwrap_or_default(),
            environment,
        })
    }

    /// Create a new client with explicit credentials (used after vault unlock)
    pub fn with_credentials(
        api_key: &str,
        account_id: &str,
        environment: OandaEnvironment,
    ) -> Result<Self> {
        let client = Client::builder()
            .min_tls_version(tls::Version::TLS_1_2)
            .build()?;

        // Mask sensitive data for logging
        let masked_account = if account_id.len() > 4 {
            format!("***{}", &account_id[account_id.len()-4..])
        } else {
            "****".to_string()
        };
        tracing::info!(
            "Creating OANDA client: env={:?}, account={}, api_key=[PRESENT]",
            environment, masked_account
        );

        Ok(Self {
            client,
            base_url: environment.api_base_url().to_string(),
            api_key: api_key.to_string(),
            account_id: account_id.to_string(),
            environment,
        })
    }

    /// Check if the client has valid credentials configured
    pub fn has_credentials(&self) -> bool {
        !self.api_key.is_empty() && !self.account_id.is_empty()
    }

    #[doc(hidden)]
    pub fn with_base_url(base_url: &str, api_key: &str, account_id: &str) -> Result<Self> {
        let client = Client::builder().build()?;

        Ok(Self {
            client,
            base_url: base_url.to_string(),
            api_key: api_key.to_string(),
            account_id: account_id.to_string(),
            environment: OandaEnvironment::Practice, // Default for tests
        })
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn environment(&self) -> OandaEnvironment {
        self.environment
    }

    pub fn get(&self, url: &str) -> reqwest::RequestBuilder {
        self.client.get(url).bearer_auth(&self.api_key)
    }

    pub fn post(&self, url: &str) -> reqwest::RequestBuilder {
        self.client
            .post(url)
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json")
    }

    pub fn put(&self, url: &str) -> reqwest::RequestBuilder {
        self.client
            .put(url)
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json")
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}
