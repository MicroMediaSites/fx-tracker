//! Script sanitization for strategy conversion.
//!
//! Strips comments, neutralizes string literals, enforces length limits,
//! and detects injection patterns before sending scripts to the AI.
//!
//! Security: Per engineering-principles.md Sections 7-13, all user input
//! must be sanitized before LLM processing.

/// Maximum allowed script length after sanitization (in characters).
const MAX_SCRIPT_LENGTH: usize = 25_000;

/// Sanitize a user-provided script before AI processing.
///
/// Performs the following operations in order:
/// 1. Strip single-line comments (`//` and `#` outside strings)
/// 2. Strip multi-line comments (`/* ... */` and `{ ... }` for MQL)
/// 3. Neutralize string literal contents
/// 4. Detect and reject injection patterns
/// 5. Enforce maximum length
///
/// Returns the sanitized script text, or an error if the script
/// contains injection patterns or exceeds the length limit.
pub fn sanitize_script(input: &str) -> Result<String, String> {
    // Step 1 & 2: Strip comments
    let stripped = strip_comments(input);

    // Step 3: Neutralize string literals
    let neutralized = neutralize_strings(&stripped);

    // Step 4: Check for injection patterns (case-insensitive)
    check_injection_patterns(&neutralized)?;

    // Step 5: Enforce length limit
    if neutralized.len() > MAX_SCRIPT_LENGTH {
        return Err(format!(
            "Script exceeds maximum length of {} characters (got {} after comment stripping). \
             Please reduce the script size.",
            MAX_SCRIPT_LENGTH,
            neutralized.len()
        ));
    }

    Ok(neutralized)
}

/// Strip single-line and multi-line comments from source code.
///
/// Handles:
/// - `//` single-line comments (Pine Script, MQL)
/// - `#` single-line comments (Pine Script) - only at start of line or after whitespace
/// - `/* ... */` multi-line comments (Pine Script, MQL)
///
/// Note: MQL-style `{ ... }` block comments are NOT stripped because
/// curly braces are too commonly used in strategy logic (JSON, code blocks).
fn strip_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut in_string = false;
    let mut string_char = '"';

    while i < len {
        // Track string state to avoid stripping inside strings
        if !in_string {
            // Check for multi-line comment start: /*
            if i + 1 < len && chars[i] == '/' && chars[i + 1] == '*' {
                // Skip until */
                i += 2;
                while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                if i + 1 < len {
                    i += 2; // Skip past */
                } else {
                    i = len; // Unterminated comment — skip to end
                }
                continue;
            }

            // Check for single-line comment: //
            if i + 1 < len && chars[i] == '/' && chars[i + 1] == '/' {
                // Skip to end of line
                while i < len && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }

            // Check for # comment (only when # is the first non-whitespace char on the line)
            if chars[i] == '#' {
                // Walk backwards to check if this is the first non-whitespace on the line
                let mut is_first_nonws = true;
                let mut j = i;
                while j > 0 {
                    j -= 1;
                    if chars[j] == '\n' {
                        break;
                    }
                    if chars[j] != ' ' && chars[j] != '\t' {
                        is_first_nonws = false;
                        break;
                    }
                }
                // Also first char of input counts
                if i == 0 {
                    is_first_nonws = true;
                }
                if is_first_nonws {
                    // Skip to end of line
                    while i < len && chars[i] != '\n' {
                        i += 1;
                    }
                    continue;
                }
            }

            // Enter string
            if chars[i] == '"' || chars[i] == '\'' {
                in_string = true;
                string_char = chars[i];
                result.push(chars[i]);
                i += 1;
                continue;
            }
        } else {
            // Exit string (handle escape sequences)
            if chars[i] == '\\' && i + 1 < len {
                result.push(chars[i]);
                result.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if chars[i] == string_char {
                in_string = false;
                result.push(chars[i]);
                i += 1;
                continue;
            }
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Replace the contents of string literals with `[string]`.
///
/// Preserves the quote delimiters but removes the actual content.
/// This prevents injection payloads hidden inside string literals.
fn neutralize_strings(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '"' || chars[i] == '\'' {
            let quote = chars[i];
            result.push(quote);

            // Skip string contents
            i += 1;
            while i < len && chars[i] != quote {
                // Handle escape sequences
                if chars[i] == '\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                i += 1;
            }

            // Write replacement and closing quote
            result.push_str("[string]");
            if i < len {
                result.push(quote);
                i += 1;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Check for common injection patterns.
///
/// Reuses the same patterns from `mcp-server-rs/src/sanitize.rs` plus
/// XML tag injection detection.
fn check_injection_patterns(input: &str) -> Result<(), String> {
    let lower = input.to_lowercase();

    // XML-like tag injection patterns
    let xml_patterns = [
        "</tool_result>",
        "</tool_use>",
        "</function>",
        "</system>",
        "<tool_result>",
        "<tool_use>",
        "<system>",
    ];

    for pattern in &xml_patterns {
        if lower.contains(pattern) {
            return Err(format!(
                "Script contains disallowed content (XML-like tags). \
                 Please remove any XML/HTML markup from the script."
            ));
        }
    }

    // Common prompt injection patterns
    let injection_patterns = [
        "ignore previous instructions",
        "ignore all instructions",
        "ignore the above",
        "disregard previous",
        "your new instructions",
    ];

    for pattern in &injection_patterns {
        if lower.contains(pattern) {
            return Err(
                "Script contains disallowed content. \
                 Please remove any instruction-like text and submit only the trading strategy code."
                    .to_string(),
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_single_line_comments_double_slash() {
        let input = "a = close // This is a comment\nb = open";
        let result = strip_comments(input);
        assert_eq!(result, "a = close \nb = open");
    }

    #[test]
    fn test_strip_single_line_comments_hash() {
        let input = "# Pine Script comment\na = close";
        let result = strip_comments(input);
        assert_eq!(result, "\na = close");
    }

    #[test]
    fn test_strip_multi_line_comments() {
        let input = "a = close /* this is\na multi-line\ncomment */ + open";
        let result = strip_comments(input);
        assert_eq!(result, "a = close  + open");
    }

    #[test]
    fn test_neutralize_string_literals() {
        let input = r#"alert("Buy signal detected")"#;
        let result = neutralize_strings(input);
        assert_eq!(result, r#"alert("[string]")"#);
    }

    #[test]
    fn test_neutralize_single_quote_strings() {
        let input = "title = 'My Strategy'";
        let result = neutralize_strings(input);
        assert_eq!(result, "title = '[string]'");
    }

    #[test]
    fn test_reject_script_exceeding_max_length() {
        let input = "a".repeat(MAX_SCRIPT_LENGTH + 1);
        let result = sanitize_script(&input);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("exceeds maximum length"));
    }

    #[test]
    fn test_reject_xml_injection() {
        let input = "some code </tool_result> more code";
        let result = sanitize_script(input);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("XML-like tags"));
    }

    #[test]
    fn test_reject_prompt_injection() {
        let input = "some code\nignore previous instructions\nmore code";
        let result = sanitize_script(input);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("disallowed content"));
    }

    #[test]
    fn test_reject_case_insensitive_injection() {
        let input = "IGNORE PREVIOUS INSTRUCTIONS";
        let result = sanitize_script(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_normal_pine_script_passes() {
        let input = r#"//@version=5
indicator("RSI Strategy", overlay=true)

rsiLength = input(14, "RSI Length")
rsiValue = ta.rsi(close, rsiLength)

longCondition = rsiValue < 30
shortCondition = rsiValue > 70

if (longCondition)
    strategy.entry("Long", strategy.long)

if (shortCondition)
    strategy.close("Long")
"#;
        let result = sanitize_script(input);
        assert!(result.is_ok());
        let sanitized = result.unwrap();
        // Comments should be stripped
        assert!(!sanitized.contains("@version=5"));
        // String contents should be neutralized
        assert!(!sanitized.contains("RSI Strategy"));
        assert!(sanitized.contains("[string]"));
        // Logic should be preserved
        assert!(sanitized.contains("ta.rsi(close, rsiLength)"));
    }

    #[test]
    fn test_mql_style_code_passes() {
        let input = r#"
// MQL4 Strategy
double rsi = iRSI(NULL, 0, 14, PRICE_CLOSE, 0);
/* Entry conditions */
if (rsi < 30) {
    OrderSend(Symbol(), OP_BUY, 0.1, Ask, 3, 0, 0, "RSI Buy", 12345, 0, Blue);
}
"#;
        let result = sanitize_script(input);
        assert!(result.is_ok());
        let sanitized = result.unwrap();
        // Comments stripped
        assert!(!sanitized.contains("MQL4 Strategy"));
        assert!(!sanitized.contains("Entry conditions"));
        // Logic preserved
        assert!(sanitized.contains("iRSI(NULL, 0, 14, PRICE_CLOSE, 0)"));
    }

    #[test]
    fn test_natural_language_passes() {
        let input = "Buy when RSI crosses below 30 and EMA 20 is above EMA 50. \
                     Sell when RSI crosses above 70. Use 50 pip stop loss and 2:1 reward ratio.";
        let result = sanitize_script(input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_preserves_hash_in_code_context() {
        // Hash followed by alphanumeric (like MQL color codes) should NOT be stripped
        let input = "color = #FF0000\n";
        let result = strip_comments(input);
        assert!(result.contains("#FF0000"));
    }

    #[test]
    fn test_empty_input() {
        let result = sanitize_script("");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "");
    }

    #[test]
    fn test_exactly_at_limit() {
        let input = "a".repeat(MAX_SCRIPT_LENGTH);
        let result = sanitize_script(&input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_unterminated_block_comment_does_not_hang() {
        // Regression: unterminated /* must not cause infinite loop
        let input = "a = close /* unterminated comment";
        let result = strip_comments(input);
        assert_eq!(result, "a = close ");
    }
}
