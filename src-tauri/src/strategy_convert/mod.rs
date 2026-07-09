//! Strategy script conversion module.
//!
//! Converts trading strategies from Pine Script, MQL4/5, or plain English
//! to wickd's V2 strategy JSON format using AI (Claude API).
//!
//! ## Architecture
//!
//! 1. User provides a script in one of the supported formats
//! 2. `sanitize::sanitize_script()` strips comments, neutralizes strings,
//!    enforces length limits, and detects injection patterns
//! 3. `convert::convert_script_to_strategy()` sends the sanitized script
//!    directly to the Claude API (runtime `ANTHROPIC_API_KEY`; the
//!    queries-service proxy was removed in AGT-650)
//! 4. The returned JSON is validated against `StrategyDefinition` via
//!    `validate_strategy_json()` before being returned to the user

pub mod convert;
pub mod sanitize;

pub use convert::{convert_script_to_strategy, validate_source_language, ConversionError};
pub use sanitize::sanitize_script;
