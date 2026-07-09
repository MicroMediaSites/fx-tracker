use reqwest::{Client, tls};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};
use crate::error::{Error, Result};

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

// Model identifiers
const MODEL_OPUS: &str = "claude-opus-4-5-20251101";
const MODEL_SONNET: &str = "claude-sonnet-4-20250514";
const MODEL_HAIKU: &str = "claude-haiku-4-5-20251022";

/// AI model selection - user picks the specific model to use
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiTier {
    Haiku,   // Fastest, lowest cost
    Sonnet,  // Balanced speed and capability
    Opus,    // Most capable
}

impl AiTier {
    pub fn model(&self) -> &'static str {
        match self {
            AiTier::Haiku => MODEL_HAIKU,
            AiTier::Sonnet => MODEL_SONNET,
            AiTier::Opus => MODEL_OPUS,
        }
    }

    /// Parse from frontend string value
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "haiku" => AiTier::Haiku,
            "sonnet" => AiTier::Sonnet,
            "opus" => AiTier::Opus,
            _ => AiTier::Opus, // Default to Opus for best quality
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClaudeClient {
    client: Client,
    api_key: String,
}

#[derive(Debug, Serialize)]
struct MessageRequest {
    model: &'static str,
    max_tokens: u32,
    messages: Vec<Message>,
    system: Option<String>,
}

/// A message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".to_string(), content: content.into() }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: "assistant".to_string(), content: content.into() }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct MessageResponse {
    content: Vec<ContentBlock>,
    #[allow(dead_code)]
    stop_reason: Option<String>,
    #[allow(dead_code)]
    model: Option<String>,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct Usage {
    input_tokens: u32,
    output_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
}

impl ClaudeClient {
    pub fn new(api_key: String) -> Result<Self> {
        let client = Client::builder()
            .min_tls_version(tls::Version::TLS_1_2)
            .build()?;

        info!("[Claude] Initialized");

        Ok(Self { client, api_key })
    }

    /// Send a message to Claude and get a response
    pub async fn send_message(
        &self,
        system_prompt: &str,
        user_message: &str,
    ) -> Result<String> {
        self.send_message_with_tier(system_prompt, user_message, None).await
    }

    /// Send a message with specific AI tier
    pub async fn send_message_with_tier(
        &self,
        system_prompt: &str,
        user_message: &str,
        tier: Option<AiTier>,
    ) -> Result<String> {
        self.send_message_with_options(system_prompt, user_message, tier, 4096).await
    }

    /// Send a message with specific AI tier and max tokens
    pub async fn send_message_with_options(
        &self,
        system_prompt: &str,
        user_message: &str,
        tier: Option<AiTier>,
        max_tokens: u32,
    ) -> Result<String> {
        // Default to Sonnet if not specified
        let tier = tier.unwrap_or(AiTier::Opus);
        let model = tier.model();
        let request = MessageRequest {
            model,
            max_tokens,
            messages: vec![Message {
                role: "user".to_string(),
                content: user_message.to_string(),
            }],
            system: Some(system_prompt.to_string()),
        };

        debug!(model, ?tier, "[Claude] Sending request");

        let response = self
            .client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(Error::Api(format!(
                "Claude API error ({}): {}",
                status, error_text
            )));
        }

        let message_response: MessageResponse = response.json().await?;

        // Log what model Anthropic actually used
        if let Some(ref response_model) = message_response.model {
            debug!(model = %response_model, "[Claude] Response received");
        }

        // Log actual token usage from API (including cache tokens)
        if let Some(usage) = &message_response.usage {
            let total_input = usage.input_tokens
                + usage.cache_creation_input_tokens
                + usage.cache_read_input_tokens;
            info!(
                input_tokens = total_input,
                output_tokens = usage.output_tokens,
                cache_write = usage.cache_creation_input_tokens,
                cache_read = usage.cache_read_input_tokens,
                "[Claude] Token usage"
            );
        }

        // Extract text from the first text content block
        let text = message_response
            .content
            .iter()
            .find(|block| block.content_type == "text")
            .and_then(|block| block.text.clone())
            .ok_or_else(|| Error::Api("No text content in Claude response".to_string()))?;

        Ok(text)
    }

    /// Check which model is actually being used by making a minimal API call
    /// Returns the model identifier from the API response metadata
    pub async fn check_model(&self, tier: AiTier) -> Result<String> {
        let model = tier.model();
        let request = MessageRequest {
            model,
            max_tokens: 10,
            messages: vec![Message {
                role: "user".to_string(),
                content: "Hi".to_string(),
            }],
            system: Some("Reply with just 'ok'".to_string()),
        };

        debug!(requested_model = model, ?tier, "[Claude] Checking model");

        let response = self
            .client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(Error::Api(format!(
                "Claude API error ({}): {}",
                status, error_text
            )));
        }

        let message_response: MessageResponse = response.json().await?;

        // Return the actual model from the API response
        let actual_model = message_response.model
            .ok_or_else(|| Error::Api("No model in response".to_string()))?;

        info!(requested = model, actual = %actual_model, "[Claude] Model check");

        Ok(actual_model)
    }

    /// Generate a trading strategy from a natural language description
    pub async fn generate_strategy(
        &self,
        system_prompt: &str,
        user_description: &str,
    ) -> Result<String> {
        let prompt = format!(
            "Generate a trading strategy based on this description:\n\n{}",
            user_description
        );

        self.send_message(system_prompt, &prompt).await
    }

    /// Continue a conversation with message history
    pub async fn chat(
        &self,
        system_prompt: &str,
        messages: Vec<ChatMessage>,
    ) -> Result<String> {
        self.chat_with_tier(system_prompt, messages, None).await
    }

    /// Continue a conversation with message history and specific AI tier
    pub async fn chat_with_tier(
        &self,
        system_prompt: &str,
        messages: Vec<ChatMessage>,
        tier: Option<AiTier>,
    ) -> Result<String> {
        // Default to Opus if not specified
        let tier = tier.unwrap_or(AiTier::Opus);
        let model = tier.model();
        info!(model, ?tier, "[Claude] Sending request to model");

        let request = MessageRequest {
            model,
            max_tokens: 4096,
            messages: messages.into_iter().map(|m| Message {
                role: m.role,
                content: m.content,
            }).collect(),
            system: Some(system_prompt.to_string()),
        };

        let response = self
            .client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(Error::Api(format!(
                "Claude API error ({}): {}",
                status, error_text
            )));
        }

        let message_response: MessageResponse = response.json().await?;

        // Log actual model used and token usage from API
        if let Some(ref response_model) = message_response.model {
            info!(model = %response_model, "[Claude] Response from model");
        }
        if let Some(usage) = &message_response.usage {
            let total_input = usage.input_tokens
                + usage.cache_creation_input_tokens
                + usage.cache_read_input_tokens;
            info!(
                input_tokens = total_input,
                output_tokens = usage.output_tokens,
                cache_write = usage.cache_creation_input_tokens,
                cache_read = usage.cache_read_input_tokens,
                "[Claude] Token usage"
            );
        }

        let text = message_response
            .content
            .iter()
            .find(|block| block.content_type == "text")
            .and_then(|block| block.text.clone())
            .ok_or_else(|| Error::Api("No text content in Claude response".to_string()))?;

        Ok(text)
    }

    /// Summarize a conversation history using Haiku (fast, cheap)
    ///
    /// Used to compress long chat histories before sending to Opus.
    /// Returns a concise summary of the conversation so far.
    pub async fn summarize_history(&self, messages: &[ChatMessage]) -> Result<String> {
        if messages.is_empty() {
            return Ok(String::new());
        }

        // Format messages for summarization
        let conversation = messages
            .iter()
            .map(|m| {
                let role = if m.role == "user" { "User" } else { "Assistant" };
                format!("{}: {}", role, m.content)
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let prompt = format!(
            "Summarize this conversation in 2-3 sentences, capturing the key topics, questions asked, and any important context or decisions. Be concise:\n\n{}",
            conversation
        );

        let system = "You are a conversation summarizer. Output ONLY the summary, no preamble.";

        info!(
            message_count = messages.len(),
            "[Claude] Summarizing history with Haiku"
        );

        self.send_message_with_options(system, &prompt, Some(AiTier::Haiku), 500).await
    }

    /// Send a help message using Haiku. Returns response with token counts for quota tracking.
    /// Used by /help command - cheaper than Opus, no tools needed.
    /// Accepts conversation history for multi-turn help conversations.
    pub async fn send_help_message(
        &self,
        system_prompt: &str,
        messages: Vec<ChatMessage>,
    ) -> Result<HelpResponse> {
        let request = MessageRequest {
            model: AiTier::Haiku.model(),
            max_tokens: 1024,
            messages: messages
                .into_iter()
                .map(|m| Message {
                    role: m.role,
                    content: m.content,
                })
                .collect(),
            system: Some(system_prompt.to_string()),
        };

        info!(
            message_count = request.messages.len(),
            "[Claude] Sending help request to Haiku"
        );

        let response = self
            .client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(Error::Api(format!(
                "Claude API error ({}): {}",
                status, error_text
            )));
        }

        let message_response: MessageResponse = response.json().await?;

        // Extract token usage
        let (input_tokens, output_tokens) = message_response
            .usage
            .map(|u| (u.input_tokens, u.output_tokens))
            .unwrap_or((0, 0));

        info!(
            input_tokens,
            output_tokens,
            "[Claude] Help response complete"
        );

        let text = message_response
            .content
            .iter()
            .find(|block| block.content_type == "text")
            .and_then(|block| block.text.clone())
            .ok_or_else(|| Error::Api("No text content in Claude response".to_string()))?;

        Ok(HelpResponse {
            text,
            input_tokens,
            output_tokens,
        })
    }
}

/// Response from help command with token counts for quota tracking
#[derive(Debug)]
pub struct HelpResponse {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
}
