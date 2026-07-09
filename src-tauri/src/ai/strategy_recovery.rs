//! AI-powered strategy error recovery
//!
//! When a strategy fails to parse or validate, this module uses AI to analyze
//! the error and suggest minimal fixes via patches. It iteratively applies fixes
//! until the strategy parses successfully or max retries is reached.

use crate::backtest::rules_engine::StrategyDefinition;
use super::client::{AiTier, ClaudeClient};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// Maximum number of fix attempts before giving up
const MAX_FIX_ATTEMPTS: usize = 5;

/// Result of AI error recovery attempt
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryResult {
    /// Human-readable explanation of what was wrong and how to fix it
    pub explanation: String,
    /// The corrected strategy JSON, if AI was able to fix it
    pub corrected_json: Option<String>,
    /// List of specific changes made
    pub changes_made: Vec<String>,
}

/// A single find/replace patch
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Patch {
    find: String,
    replace: String,
}

/// Raw response from AI (patches, not full JSON)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PatchResponse {
    explanation: String,
    patches: Vec<Patch>,
}

/// System prompt for strategy error recovery
const RECOVERY_SYSTEM_PROMPT: &str = r#"You are a JSON repair tool for trading strategy configurations.

Your ONLY job is to fix parsing errors. You must NOT:
- Add new indicators, rules, or features
- Improve the strategy logic
- Suggest trading improvements
- Change anything beyond what's needed to fix the error

You MUST:
- Return ONLY the minimal text replacements needed to fix the error
- Each patch is a find/replace pair - find the exact text and replace it
- If the error mentions valid options, use the first one listed
- Keep patches as small as possible - just the broken part, not large chunks"#;

/// User prompt template for recovery - patch-based
const RECOVERY_USER_PROMPT: &str = r#"A strategy failed to parse with this error:
{error}

Strategy JSON:
{strategy_json}

Return a JSON object with find/replace patches to fix the error (no markdown, just raw JSON):
{
  "explanation": "Brief description of what was wrong and how you fixed it",
  "patches": [
    {"find": "exact text to find", "replace": "replacement text"}
  ]
}

CRITICAL RULES:
- Use the SMALLEST possible find string that uniquely identifies what to change
- For type errors like this one, just use: "type":"decimal" -> "type":"number" (no extra fields!)
- Do NOT include surrounding fields - JSON field order varies and will cause mismatches
- Each "find" must be an EXACT substring that exists in the JSON above
- If you cannot fix the error, return empty patches array and explain in explanation"#;

/// Attempt to recover a strategy from a parsing error using AI
/// Iteratively applies fixes until the strategy parses successfully
pub async fn recover_strategy_error(
    client: &ClaudeClient,
    error: &str,
    strategy_json: &str,
) -> Result<RecoveryResult, String> {
    let mut current_json = strategy_json.to_string();
    let mut current_error = error.to_string();
    let mut all_changes: Vec<String> = Vec::new();
    let mut last_explanation = String::new();

    for attempt in 1..=MAX_FIX_ATTEMPTS {
        info!(attempt, error = %current_error, "AI recovery attempt");

        // Try to get patches from AI
        let patch_result = get_patches(client, &current_error, &current_json).await?;
        last_explanation = patch_result.explanation.clone();

        if patch_result.patches.is_empty() {
            info!("AI returned no patches, giving up");
            break;
        }

        // Apply patches
        let (patched_json, changes) = apply_patches(&current_json, &patch_result.patches);
        all_changes.extend(changes);
        current_json = patched_json;

        // Try to parse as StrategyDefinition
        match serde_json::from_str::<StrategyDefinition>(&current_json) {
            Ok(_) => {
                info!(attempt, total_changes = all_changes.len(), "Strategy fixed successfully!");

                // Validate minimal changes
                validate_minimal_changes(strategy_json, &current_json)?;

                return Ok(RecoveryResult {
                    explanation: last_explanation,
                    corrected_json: Some(current_json),
                    changes_made: all_changes,
                });
            }
            Err(e) => {
                let new_error = format!("Failed to parse strategy JSON: {}", e);
                info!(attempt, new_error = %new_error, "Strategy still has errors, continuing...");
                current_error = new_error;
            }
        }
    }

    // Exhausted retries - return what we have
    warn!(
        attempts = MAX_FIX_ATTEMPTS,
        "Could not fully fix strategy after max attempts"
    );

    Ok(RecoveryResult {
        explanation: format!("{} (Could not fully resolve all errors after {} attempts)",
            last_explanation, MAX_FIX_ATTEMPTS),
        corrected_json: None,
        changes_made: all_changes,
    })
}

/// Get patches from AI for a single error
async fn get_patches(
    client: &ClaudeClient,
    error: &str,
    strategy_json: &str,
) -> Result<PatchResponse, String> {
    let user_prompt = RECOVERY_USER_PROMPT
        .replace("{error}", error)
        .replace("{strategy_json}", strategy_json);

    let response = client
        .send_message_with_options(RECOVERY_SYSTEM_PROMPT, &user_prompt, Some(AiTier::Haiku), 4096)
        .await
        .map_err(|e| format!("AI recovery failed: {}", e))?;

    debug!(response_len = response.len(), "Received AI response");
    info!("AI patch response: {}", &response[..response.len().min(300)]);

    let json_str = extract_json(&response);

    serde_json::from_str(json_str)
        .map_err(|e| format!("Failed to parse AI response: {}", e))
}

/// Apply patches to JSON and return the result with list of changes made
fn apply_patches(json: &str, patches: &[Patch]) -> (String, Vec<String>) {
    let mut result = json.to_string();
    let mut changes = Vec::new();

    for patch in patches {
        if result.contains(&patch.find) {
            result = result.replace(&patch.find, &patch.replace);
            changes.push(format!("Changed '{}' to '{}'",
                truncate_for_display(&patch.find, 30),
                truncate_for_display(&patch.replace, 30)
            ));
            info!("Applied patch: {} -> {}",
                truncate_for_display(&patch.find, 50),
                truncate_for_display(&patch.replace, 50)
            );
        } else {
            warn!("Patch target not found: {}", truncate_for_display(&patch.find, 100));
        }
    }

    (result, changes)
}

/// Truncate a string for display in logs
fn truncate_for_display(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

/// Extract JSON from a response that might be wrapped in markdown code blocks
fn extract_json(response: &str) -> &str {
    let trimmed = response.trim();

    // Try to find JSON code block
    if let Some(start) = trimmed.find("```json") {
        let json_start = start + 7;
        if let Some(end) = trimmed[json_start..].find("```") {
            return trimmed[json_start..json_start + end].trim();
        }
    }

    // Try generic code block
    if let Some(start) = trimmed.find("```") {
        let code_start = start + 3;
        // Skip language identifier if present (e.g., ```json\n)
        let actual_start = trimmed[code_start..]
            .find('\n')
            .map(|i| code_start + i + 1)
            .unwrap_or(code_start);
        if let Some(end) = trimmed[actual_start..].find("```") {
            return trimmed[actual_start..actual_start + end].trim();
        }
    }

    // No code block, return as-is
    trimmed
}

/// Validate that the AI didn't make too many changes (abuse prevention)
/// Note: We allow significant structural changes because converting string expressions
/// to structured objects legitimately adds many fields.
fn validate_minimal_changes(original: &str, corrected: &str) -> Result<(), String> {
    // Parse both to compare structure
    let orig_value: serde_json::Value = serde_json::from_str(original)
        .map_err(|_| "Original JSON is invalid")?;
    let corr_value: serde_json::Value = serde_json::from_str(corrected)
        .map_err(|_| "Corrected JSON is invalid")?;

    // Count fields - allow up to 50 field difference for expression conversions
    let orig_fields = count_fields(&orig_value);
    let corr_fields = count_fields(&corr_value);

    let field_diff = (orig_fields as i32 - corr_fields as i32).abs();
    if field_diff > 50 {
        warn!(
            orig_fields,
            corr_fields,
            field_diff,
            "AI made very significant structural changes"
        );
        return Err("AI made too many structural changes - manual review required".to_string());
    }

    // Log but don't block on moderate changes
    if field_diff > 10 {
        info!(
            orig_fields,
            corr_fields,
            field_diff,
            "AI made moderate structural changes (likely expression conversion)"
        );
    }

    Ok(())
}

/// Recursively count fields in a JSON value
fn count_fields(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Object(map) => {
            map.len() + map.values().map(count_fields).sum::<usize>()
        }
        serde_json::Value::Array(arr) => arr.iter().map(count_fields).sum(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_plain() {
        let input = r#"{"explanation": "test", "corrected_json": null, "changes_made": []}"#;
        assert_eq!(extract_json(input), input);
    }

    #[test]
    fn test_extract_json_markdown() {
        let input = r#"```json
{"explanation": "test", "corrected_json": null, "changes_made": []}
```"#;
        let expected = r#"{"explanation": "test", "corrected_json": null, "changes_made": []}"#;
        assert_eq!(extract_json(input), expected);
    }

    #[test]
    fn test_validate_minimal_changes_ok() {
        let original = r#"{"name": "test", "type": "decimal"}"#;
        let corrected = r#"{"name": "test", "type": "number"}"#;
        assert!(validate_minimal_changes(original, corrected).is_ok());
    }

    #[test]
    fn test_validate_minimal_changes_too_many() {
        // Threshold is 50 fields, so we need to add more than 50
        let original = r#"{"name": "test"}"#;
        // Build a JSON with 60 extra fields
        let mut corrected = String::from(r#"{"name": "test""#);
        for i in 0..60 {
            corrected.push_str(&format!(r#", "field{}": {}"#, i, i));
        }
        corrected.push('}');
        assert!(validate_minimal_changes(original, &corrected).is_err());
    }

    #[test]
    fn test_count_fields() {
        let value: serde_json::Value = serde_json::from_str(
            r#"{"a": 1, "b": {"c": 2, "d": 3}, "e": [{"f": 4}]}"#
        ).unwrap();
        // a, b, b.c, b.d, e, e[0].f = 6 fields total
        assert_eq!(count_fields(&value), 6);
    }
}
