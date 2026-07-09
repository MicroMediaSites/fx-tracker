//! Strategy conversion via AI (direct Anthropic Claude API).
//!
//! AGT-650: the queries-service proxy (and its Clerk auth + quota plumbing)
//! was removed. Conversion now calls Claude directly through the app's
//! [`ClaudeClient`] (runtime `ANTHROPIC_API_KEY`), using the same system
//! prompt the proxy served (`conversion-prompt.md`, kept in this module).

use crate::ai::{AiTier, ClaudeClient};
use tracing::info;

/// System prompt for strategy conversion. Loaded at compile time — never
/// accepted from the frontend (prevents system-prompt injection).
const CONVERSION_SYSTEM_PROMPT: &str = include_str!("conversion-prompt.md");

/// Errors that can occur during strategy conversion.
#[derive(Debug)]
pub enum ConversionError {
    /// The source language is not supported
    InvalidSourceLanguage(String),
    /// The AI request failed
    RequestFailed(String),
    /// The AI returned a response that isn't valid JSON
    InvalidResponse(String),
}

impl std::fmt::Display for ConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConversionError::InvalidSourceLanguage(lang) => {
                write!(f, "Unsupported source language: '{}'. Use one of: pine_script, mql4, mql5, natural_language", lang)
            }
            ConversionError::RequestFailed(msg) => {
                write!(f, "Conversion request failed: {}", msg)
            }
            ConversionError::InvalidResponse(msg) => {
                write!(f, "AI returned invalid response: {}", msg)
            }
        }
    }
}

/// Supported source languages for conversion.
const VALID_LANGUAGES: &[&str] = &["pine_script", "mql4", "mql5", "natural_language"];

/// Validate that a source language string is supported.
pub fn validate_source_language(language: &str) -> Result<(), ConversionError> {
    if VALID_LANGUAGES.contains(&language) {
        Ok(())
    } else {
        Err(ConversionError::InvalidSourceLanguage(language.to_string()))
    }
}

/// Convert a sanitized script to wickd strategy JSON via AI.
///
/// # Arguments
/// * `script` - The sanitized script text
/// * `source_language` - One of: "pine_script", "mql4", "mql5", "natural_language"
/// * `claude` - The app's direct Anthropic client
///
/// # Returns
/// The raw strategy JSON string from the AI response.
pub async fn convert_script_to_strategy(
    script: &str,
    source_language: &str,
    claude: &ClaudeClient,
) -> Result<String, ConversionError> {
    validate_source_language(source_language)?;

    let language_label = match source_language {
        "pine_script" => "Pine Script (TradingView)",
        "mql4" => "MQL4 (MetaTrader 4)",
        "mql5" => "MQL5 (MetaTrader 5)",
        "natural_language" => "plain English description",
        _ => source_language,
    };

    // Build the user message with the script
    let user_message = format!(
        "Convert the following {} trading strategy to wickd JSON format.\n\n\
         Source script:\n```\n{}\n```\n\n\
         Return ONLY the JSON object, no markdown fences or explanations.",
        language_label, script
    );

    info!(
        source_language = source_language,
        script_len = script.len(),
        "[StrategyConvert] Sending conversion request"
    );

    // Same model + token budget the queries-service endpoint used
    // (Sonnet, 8192 max tokens).
    let text = claude
        .send_message_with_options(
            CONVERSION_SYSTEM_PROMPT,
            &user_message,
            Some(AiTier::Sonnet),
            8192,
        )
        .await
        .map_err(|e| ConversionError::RequestFailed(e.to_string()))?;

    info!(
        response_len = text.len(),
        "[StrategyConvert] Conversion response received"
    );

    // Extract JSON from the response (Claude may wrap it in markdown fences)
    let json_text = extract_json(&text)?;

    Ok(json_text)
}

/// Extract JSON from AI response text.
///
/// Handles cases where Claude wraps the JSON in markdown code fences.
fn extract_json(text: &str) -> Result<String, ConversionError> {
    let trimmed = text.trim();

    // Try direct parse first
    if trimmed.starts_with('{') {
        // Validate it's actually parseable JSON
        serde_json::from_str::<serde_json::Value>(trimmed)
            .map_err(|e| ConversionError::InvalidResponse(format!("Invalid JSON: {}", e)))?;
        return Ok(trimmed.to_string());
    }

    // Try extracting from markdown code fence
    if let Some(start) = trimmed.find("```json") {
        let json_start = start + 7; // skip ```json
        if let Some(end) = trimmed[json_start..].find("```") {
            let json = trimmed[json_start..json_start + end].trim();
            serde_json::from_str::<serde_json::Value>(json)
                .map_err(|e| ConversionError::InvalidResponse(format!("Invalid JSON in code block: {}", e)))?;
            return Ok(json.to_string());
        }
    }

    // Try extracting from plain code fence
    if let Some(start) = trimmed.find("```") {
        let fence_start = start + 3;
        // Skip language identifier if present (e.g., ```json\n)
        let content_start = if let Some(newline) = trimmed[fence_start..].find('\n') {
            fence_start + newline + 1
        } else {
            fence_start
        };
        if let Some(end) = trimmed[content_start..].find("```") {
            let json = trimmed[content_start..content_start + end].trim();
            if json.starts_with('{') {
                serde_json::from_str::<serde_json::Value>(json)
                    .map_err(|e| ConversionError::InvalidResponse(format!("Invalid JSON in code block: {}", e)))?;
                return Ok(json.to_string());
            }
        }
    }

    Err(ConversionError::InvalidResponse(
        "AI response does not contain valid strategy JSON. The response must be a JSON object starting with '{'.".to_string()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_source_language_valid() {
        assert!(validate_source_language("pine_script").is_ok());
        assert!(validate_source_language("mql4").is_ok());
        assert!(validate_source_language("mql5").is_ok());
        assert!(validate_source_language("natural_language").is_ok());
    }

    #[test]
    fn test_validate_source_language_invalid() {
        let result = validate_source_language("python");
        assert!(matches!(result, Err(ConversionError::InvalidSourceLanguage(_))));

        let result = validate_source_language("");
        assert!(matches!(result, Err(ConversionError::InvalidSourceLanguage(_))));

        let result = validate_source_language("PineScript");
        assert!(matches!(result, Err(ConversionError::InvalidSourceLanguage(_))));
    }

    #[test]
    fn test_conversion_error_display() {
        let err = ConversionError::InvalidSourceLanguage("python".to_string());
        assert!(err.to_string().contains("python"));
        assert!(err.to_string().contains("pine_script"));

        let err = ConversionError::RequestFailed("timeout".to_string());
        assert!(err.to_string().contains("timeout"));

        let err = ConversionError::RequestFailed("auth".to_string());
        assert!(err.to_string().contains("auth"));
    }

    #[test]
    fn test_extract_json_direct() {
        let input = r#"{"schema_version": 2, "name": "Test"}"#;
        let result = extract_json(input);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), input);
    }

    #[test]
    fn test_extract_json_with_markdown_fence() {
        let input = "Here's the strategy:\n```json\n{\"schema_version\": 2}\n```\nDone.";
        let result = extract_json(input);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "{\"schema_version\": 2}");
    }

    #[test]
    fn test_extract_json_with_plain_fence() {
        let input = "```\n{\"schema_version\": 2}\n```";
        let result = extract_json(input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_extract_json_with_whitespace() {
        let input = "  \n  {\"schema_version\": 2}  \n  ";
        let result = extract_json(input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_extract_json_no_json() {
        let input = "This is just plain text with no JSON.";
        let result = extract_json(input);
        assert!(matches!(result, Err(ConversionError::InvalidResponse(_))));
    }

    #[test]
    fn test_extract_json_invalid_json() {
        let input = "{invalid json here}";
        let result = extract_json(input);
        assert!(matches!(result, Err(ConversionError::InvalidResponse(_))));
    }

}
