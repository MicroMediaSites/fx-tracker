use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("OANDA API error: {0}")]
    OandaApi(String),

    #[error("API error: {0}")]
    Api(String),

    #[error("Environment error: {0}")]
    Env(#[from] std::env::VarError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    #[error("Strategy error: {0}")]
    Strategy(String),

    #[error("Cryptography error: {0}")]
    Crypto(String),

    #[error("Authentication error: {0}")]
    Auth(String),
}

pub type Result<T> = std::result::Result<T, Error>;
