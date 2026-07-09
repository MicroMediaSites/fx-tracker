//! Tool parameter types
//!
//! Input parameter structs for MCP tool calls.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ============================================================================
// Bounds validation constants
// ============================================================================

/// Maximum allowed limit for any query (prevents OOM/DoS)
pub const MAX_LIMIT: i32 = 1000;

/// Maximum allowed content length for notes (50KB)
pub const MAX_CONTENT_LENGTH: usize = 50 * 1024;

/// Maximum allowed title length
pub const MAX_TITLE_LENGTH: usize = 500;

// ============================================================================
// Default value helpers
// ============================================================================

pub fn default_limit_50() -> i32 { 50 }
pub fn default_limit_20() -> i32 { 20 }
pub fn default_limit_10() -> i32 { 10 }

/// Clamp a limit value to the allowed range [1, MAX_LIMIT]
pub fn clamp_limit(limit: i32) -> i32 {
    limit.clamp(1, MAX_LIMIT)
}

/// Maximum allowed ID length (prevents abuse with huge strings)
const MAX_ID_LENGTH: usize = 256;

/// Validate that an ID string is non-empty and reasonable length (AUDIT-014).
/// IDs can be UUIDs, prefixed strings (strat_*), OANDA trade IDs, etc.
pub fn validate_id(id: &str, field_name: &str) -> Option<String> {
    if id.is_empty() {
        Some(format!("Invalid {}: must not be empty", field_name))
    } else if id.len() > MAX_ID_LENGTH {
        Some(format!("Invalid {}: exceeds maximum length", field_name))
    } else {
        None
    }
}

/// Validate an optional ID field
pub fn validate_optional_id(id: Option<&String>, field_name: &str) -> Option<String> {
    id.and_then(|i| validate_id(i, field_name))
}

/// Validate a price string is a valid positive decimal (AUDIT-014)
pub fn validate_price(price: &str, field_name: &str) -> Result<rust_decimal::Decimal, String> {
    use rust_decimal::Decimal;
    use std::str::FromStr;

    let parsed = Decimal::from_str(price)
        .map_err(|_| format!("Invalid {}: must be a valid decimal number", field_name))?;

    if parsed <= Decimal::ZERO {
        return Err(format!("Invalid {}: must be a positive number", field_name));
    }

    Ok(parsed)
}

/// Validate upper and lower prices are valid and upper > lower (AUDIT-014)
pub fn validate_price_range(upper: &str, lower: &str) -> Result<(rust_decimal::Decimal, rust_decimal::Decimal), String> {
    let upper_price = validate_price(upper, "upper_price")?;
    let lower_price = validate_price(lower, "lower_price")?;

    if lower_price >= upper_price {
        return Err("Invalid price range: upper_price must be greater than lower_price".to_string());
    }

    Ok((upper_price, lower_price))
}

// ============================================================================
// Strategy Parameters
// ============================================================================

/// Parameters for creating a new strategy
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CreateStrategyParams {
    /// Name of the strategy
    pub name: String,
    /// Description of the strategy
    #[serde(default)]
    pub description: Option<String>,
    /// Array of indicator configurations: [{id, type, params: {period, ...}}]
    pub indicators: serde_json::Value,
    /// Array of parameter definitions for optimization (optional)
    #[serde(default)]
    pub parameters: Option<serde_json::Value>,
    /// Array of variable definitions (optional)
    #[serde(default)]
    pub variables: Option<serde_json::Value>,
    /// Array of entry rules with conditions and triggers
    pub entry_rules: serde_json::Value,
    /// Array of exit rules (can be empty [])
    pub exit_rules: serde_json::Value,
    /// Risk settings object: {risk_method, risk_value, rr_ratio, spread_buffer_pips}
    pub risk_settings: serde_json::Value,
}

/// Parameters for updating a strategy
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct UpdateStrategyParams {
    /// ID of the strategy to update
    pub id: String,
    /// New name (optional)
    #[serde(default)]
    pub name: Option<String>,
    /// New description (optional)
    #[serde(default)]
    pub description: Option<String>,
    /// New indicator configurations (optional)
    #[serde(default)]
    pub indicators: Option<serde_json::Value>,
    /// New parameter definitions (optional)
    #[serde(default)]
    pub parameters: Option<serde_json::Value>,
    /// New variable definitions (optional)
    #[serde(default)]
    pub variables: Option<serde_json::Value>,
    /// New entry rules (optional)
    #[serde(default)]
    pub entry_rules: Option<serde_json::Value>,
    /// New exit rules (optional)
    #[serde(default)]
    pub exit_rules: Option<serde_json::Value>,
    /// New risk settings (optional)
    #[serde(default)]
    pub risk_settings: Option<serde_json::Value>,
}

/// Parameters for getting a specific strategy
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetStrategyParams {
    /// ID of the strategy to retrieve
    pub id: String,
}

// ============================================================================
// Trade Parameters
// ============================================================================

/// Parameters for getting account summary
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetAccountSummaryParams {
    /// Start date (ISO 8601 string like "2023-01-01" or epoch milliseconds)
    #[serde(default)]
    pub date_from: Option<String>,
    /// End date (ISO 8601 string like "2023-12-31" or epoch milliseconds)
    #[serde(default)]
    pub date_to: Option<String>,
}

/// Parameters for getting trades
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetTradesParams {
    /// Filter by instrument (e.g., EUR_USD)
    #[serde(default)]
    pub instrument: Option<String>,
    /// Filter by state (OPEN or CLOSED)
    #[serde(default)]
    pub state: Option<String>,
    /// Start date (ISO 8601 string like "2023-01-01" or epoch milliseconds)
    #[serde(default)]
    pub date_from: Option<String>,
    /// End date (ISO 8601 string like "2023-12-31" or epoch milliseconds)
    #[serde(default)]
    pub date_to: Option<String>,
    /// Maximum trades to return (default 50)
    #[serde(default = "default_limit_50")]
    pub limit: i32,
}

// ============================================================================
// Note Parameters
// ============================================================================

/// Parameters for getting notes
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetNotesParams {
    /// Filter by associated trade ID
    #[serde(default)]
    pub trade_id: Option<String>,
    /// Filter by associated strategy ID
    #[serde(default)]
    pub strategy_id: Option<String>,
    /// Search in title and content
    #[serde(default)]
    pub search: Option<String>,
    /// Maximum results to return (default 20)
    #[serde(default = "default_limit_20")]
    pub limit: i32,
}

/// Parameters for creating a note
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CreateNoteParams {
    /// Note title
    pub title: String,
    /// Note content (supports markdown)
    pub content: String,
    /// Optional trade ID to associate with
    #[serde(default)]
    pub trade_id: Option<String>,
    /// Optional strategy ID to associate with
    #[serde(default)]
    pub strategy_id: Option<String>,
}

/// Parameters for updating a note
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct UpdateNoteParams {
    /// The note ID to update
    pub note_id: String,
    /// New title
    #[serde(default)]
    pub title: Option<String>,
    /// New content
    #[serde(default)]
    pub content: Option<String>,
}

// ============================================================================
// Zone Parameters
// ============================================================================

/// Parameters for getting S/R zones
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetZonesParams {
    /// The instrument (e.g., EUR_USD)
    pub instrument: String,
}

/// Parameters for creating an S/R zone
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CreateZoneParams {
    /// The instrument (e.g., EUR_USD)
    pub instrument: String,
    /// Upper boundary price
    pub upper_price: String,
    /// Lower boundary price
    pub lower_price: String,
    /// Optional label for the zone
    #[serde(default)]
    pub label: Option<String>,
    /// Optional color (hex format)
    #[serde(default)]
    pub color: Option<String>,
}

/// Parameters for deleting an S/R zone
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DeleteZoneParams {
    /// The zone ID to delete
    pub zone_id: String,
    /// Out-of-band confirmation nonce. Leave unset on the first call: the server
    /// returns a nonce and an ack-file path, and a HUMAN must create that file
    /// before the deletion will run. On the follow-up call, echo the nonce here.
    /// A simple boolean flag is intentionally NOT accepted — the model cannot
    /// self-confirm a destructive action.
    #[serde(default)]
    pub confirm_token: Option<String>,
}

// ============================================================================
// Backtest Parameters
// ============================================================================

/// Parameters for getting backtests
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetBacktestsParams {
    /// The strategy ID
    pub strategy_id: String,
    /// Maximum results to return (default 10)
    #[serde(default = "default_limit_10")]
    pub limit: i32,
}

// ============================================================================
// Help Parameters
// ============================================================================

/// Parameters for getting help by topic
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetHelpParams {
    /// Topic name (e.g., "strategy-authoring", "indicators", "backtesting")
    pub topic: String,
}

// ============================================================================
// Conversion Parameters
// ============================================================================

/// Maximum allowed script length for conversion (10KB)
pub const MAX_SCRIPT_LENGTH: usize = 10_000;

/// Parameters for converting a trading strategy script to wickd JSON
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ConvertStrategyParams {
    /// The script text to convert (Pine Script, MQL4, MQL5, or plain English)
    pub script: String,
    /// The source language: "pine_script", "mql4", "mql5", or "natural_language"
    pub source_language: String,
}
