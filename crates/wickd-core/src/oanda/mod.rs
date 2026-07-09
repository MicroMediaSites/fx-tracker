//! OANDA API integration module
//!
//! This module contains all the code for interacting with the OANDA REST API.
//! It's organized into submodules:
//! - `client`: The HTTP client wrapper with authentication
//! - `types`: OANDA API response types (their exact JSON structure)
//! - `endpoints`: Functions for each API endpoint
//! - `streaming`: Real-time price streaming

pub mod client;
pub mod types;
pub mod endpoints;
pub mod streaming;

// Re-export the main client for convenience
pub use client::OandaClient;
pub use streaming::{PriceStreamer, PriceUpdate};
pub use endpoints::Granularity;
