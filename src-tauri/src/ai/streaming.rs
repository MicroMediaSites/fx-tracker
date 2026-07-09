//! Anthropic SSE streaming support for Claude API
//!
//! Handles Server-Sent Events (SSE) streaming from the Anthropic Messages API.
//! Emits token events to the frontend via Tauri's event system.
//!
//! Supports tool use: when Claude decides to use a tool, the stream returns
//! with ToolUseRequested, allowing the caller to execute the tool and continue.

use futures_util::StreamExt;
use reqwest::{Client, tls};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::io::StreamReader;
use tracing::{debug, info, warn};

use super::client::{AiTier, ChatMessage};
use crate::error::{Error, Result};

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

// ============================================================================
// Tauri Event Payloads
// ============================================================================

/// Emitted for each text chunk received from the API
#[derive(Debug, Clone, Serialize)]
pub struct ChatTokenEvent {
    pub session_id: String,
    pub text: String,
}

/// Emitted when the stream completes successfully
#[derive(Debug, Clone, Serialize)]
pub struct ChatCompleteEvent {
    pub session_id: String,
    pub full_text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Stop reason: "end_turn" (normal), "max_tokens" (truncated), "stop_sequence", "tool_use"
    pub stop_reason: Option<String>,
}

/// Emitted on stream error
#[derive(Debug, Clone, Serialize)]
pub struct ChatErrorEvent {
    pub session_id: String,
    pub error_type: String,
    pub message: String,
}

// ============================================================================
// Tool Use Types
// ============================================================================

/// Information about a tool use request from Claude
#[derive(Debug, Clone, Serialize)]
pub struct ToolUseRequest {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// Result of streaming - either completed or needs tool execution
#[derive(Debug)]
pub enum StreamResult {
    /// Stream completed with final text
    Complete {
        full_text: String,
        input_tokens: u32,
        output_tokens: u32,
        /// Stop reason from API: "end_turn", "max_tokens", "stop_sequence", "tool_use"
        stop_reason: Option<String>,
    },
    /// Claude requested tool use - caller should execute and continue
    ToolUse {
        /// Text generated before the tool call (if any)
        text_so_far: String,
        /// Tool that was requested
        tool_request: ToolUseRequest,
        /// Tokens used so far
        input_tokens: u32,
        output_tokens: u32,
    },
    /// User cancelled
    Cancelled,
}

// ============================================================================
// Anthropic SSE Types
// ============================================================================

/// Cache control for prompt caching
#[derive(Debug, Clone, Serialize)]
pub struct CacheControl {
    #[serde(rename = "type")]
    pub control_type: String,
}

impl CacheControl {
    pub fn ephemeral() -> Self {
        Self {
            control_type: "ephemeral".to_string(),
        }
    }
}

/// System prompt block with optional cache control
#[derive(Debug, Clone, Serialize)]
pub struct SystemBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

impl SystemBlock {
    pub fn text(content: &str) -> Self {
        Self {
            block_type: "text".to_string(),
            text: content.to_string(),
            cache_control: None,
        }
    }

    pub fn text_cached(content: &str) -> Self {
        Self {
            block_type: "text".to_string(),
            text: content.to_string(),
            cache_control: Some(CacheControl::ephemeral()),
        }
    }
}

/// Request body for streaming messages with optional tools
#[derive(Debug, Serialize)]
struct StreamingMessageRequest<'a> {
    model: &'static str,
    max_tokens: u32,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<Vec<SystemBlock>>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ApiToolWithCache>>,
    #[serde(skip)]
    _phantom: std::marker::PhantomData<&'a ()>,
}

/// Tool definition in Anthropic's format (with optional cache control)
#[derive(Debug, Clone, Serialize)]
pub struct ApiToolWithCache {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

/// Tool definition in Anthropic's format (public API without cache control)
#[derive(Debug, Clone, Serialize)]
pub struct ApiTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Message for the API - supports both simple text and structured content
#[derive(Debug, Clone, Serialize)]
pub struct ApiMessage {
    pub role: String,
    pub content: ApiContent,
}

/// Content can be simple text or structured blocks
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ApiContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

/// Content block types for structured messages
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

/// SSE event: message_start
#[derive(Debug, Deserialize)]
struct MessageStartEvent {
    message: MessageMeta,
}

#[derive(Debug, Deserialize)]
struct MessageMeta {
    #[allow(dead_code)]
    id: String,
    model: String,
    usage: UsageStart,
}

#[derive(Debug, Deserialize)]
struct UsageStart {
    input_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
}

/// SSE event: content_block_start
#[derive(Debug, Deserialize)]
struct ContentBlockStartEvent {
    #[allow(dead_code)]
    index: usize,
    content_block: ContentBlockStart,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ContentBlockStart {
    #[serde(rename = "text")]
    Text {
        #[allow(dead_code)]
        text: String,
    },
    #[serde(rename = "tool_use")]
    ToolUse { id: String, name: String },
}

/// SSE event: content_block_delta
#[derive(Debug, Deserialize)]
struct ContentBlockDeltaEvent {
    #[allow(dead_code)]
    index: usize,
    delta: Delta,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum Delta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
}

/// SSE event: message_delta (final usage stats and stop reason)
#[derive(Debug, Deserialize)]
struct MessageDeltaEvent {
    delta: MessageDelta,
    usage: UsageDelta,
}

#[derive(Debug, Deserialize)]
struct MessageDelta {
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsageDelta {
    output_tokens: u32,
}

/// SSE event: error
#[derive(Debug, Deserialize)]
struct ErrorEvent {
    error: ApiError,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    #[serde(rename = "type")]
    error_type: String,
    message: String,
}

// ============================================================================
// Active Content Block Tracking
// ============================================================================

/// Tracks the current content block being streamed
#[derive(Debug)]
enum ActiveBlock {
    Text,
    ToolUse {
        id: String,
        name: String,
        input_json: String,
    },
}

// ============================================================================
// Streaming Client
// ============================================================================

/// Client for streaming chat with Anthropic's Claude API
#[derive(Debug, Clone)]
pub struct StreamingClaudeClient {
    client: Client,
    api_key: String,
}

impl StreamingClaudeClient {
    pub fn new(api_key: String) -> Result<Self> {
        let client = Client::builder()
            .min_tls_version(tls::Version::TLS_1_2)
            .build()?;

        Ok(Self { client, api_key })
    }

    /// Stream a chat conversation, emitting tokens as they arrive
    ///
    /// Returns StreamResult indicating whether the stream completed or
    /// if a tool needs to be executed.
    ///
    /// # Arguments
    /// * `system_prompt` - System prompt for the conversation
    /// * `messages` - Conversation history (can include tool use/result blocks)
    /// * `tools` - Optional tools to make available
    /// * `tier` - AI model tier to use
    /// * `app_handle` - Tauri app handle for emitting events
    /// * `session_id` - Unique session ID for this stream
    /// * `cancel_token` - Atomic bool to signal cancellation
    pub async fn stream_chat_with_tools(
        &self,
        system_prompt: &str,
        messages: Vec<ApiMessage>,
        tools: Option<&[ApiTool]>,
        tier: Option<AiTier>,
        app_handle: AppHandle,
        session_id: String,
        cancel_token: Arc<AtomicBool>,
    ) -> Result<StreamResult> {
        let tier = tier.unwrap_or(AiTier::Opus);
        let model = tier.model();

        info!(
            model,
            session_id = %session_id,
            has_tools = tools.is_some(),
            "[StreamChat] Starting stream with prompt caching"
        );

        // Build system prompt with cache control for prompt caching
        // The system prompt is marked for caching so it doesn't count against rate limits
        let system_blocks = Some(vec![SystemBlock::text_cached(system_prompt)]);

        // Build tools with cache control on the last tool
        // This caches all tool definitions (cache order: tools → system → messages)
        let tools_with_cache: Option<Vec<ApiToolWithCache>> = tools.map(|tool_slice| {
            let len = tool_slice.len();
            tool_slice
                .iter()
                .enumerate()
                .map(|(i, t)| ApiToolWithCache {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    input_schema: t.input_schema.clone(),
                    // Add cache_control to the last tool only
                    cache_control: if i == len - 1 {
                        Some(CacheControl::ephemeral())
                    } else {
                        None
                    },
                })
                .collect()
        });

        let request = StreamingMessageRequest {
            model,
            max_tokens: 8192,
            messages,
            system: system_blocks,
            stream: true,
            tools: tools_with_cache,
            _phantom: std::marker::PhantomData,
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
            let error_msg = format!("Claude API error ({}): {}", status, error_text);

            let _ = app_handle.emit(
                "chat-error",
                ChatErrorEvent {
                    session_id,
                    error_type: "api_error".to_string(),
                    message: error_msg.clone(),
                },
            );

            return Err(Error::Api(error_msg));
        }

        // Process SSE stream
        let stream = response.bytes_stream();
        let stream = stream.map(|result| {
            result.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
        });
        let reader = StreamReader::new(stream);
        let mut lines = BufReader::new(reader).lines();

        let mut accumulated_text = String::new();
        let mut input_tokens: u32 = 0;
        let mut output_tokens: u32 = 0;
        let mut current_event_type: Option<String> = None;
        let mut active_block: Option<ActiveBlock> = None;
        let mut pending_tool_use: Option<ToolUseRequest> = None;
        let mut stop_reason: Option<String> = None;

        loop {
            // Check for cancellation
            if cancel_token.load(Ordering::SeqCst) {
                info!(session_id = %session_id, "[StreamChat] Cancelled by user");
                return Ok(StreamResult::Cancelled);
            }

            match lines.next_line().await {
                Ok(Some(line)) => {
                    let line = line.trim();

                    if line.is_empty() {
                        continue;
                    }

                    // SSE format: "event: <event_type>" followed by "data: <json>"
                    if let Some(event_type) = line.strip_prefix("event: ") {
                        current_event_type = Some(event_type.to_string());
                        continue;
                    }

                    if let Some(data) = line.strip_prefix("data: ") {
                        if let Some(ref event_type) = current_event_type {
                            match event_type.as_str() {
                                "message_start" => {
                                    if let Ok(event) = serde_json::from_str::<MessageStartEvent>(data) {
                                        // Sum all input token types (regular + cache write + cache read)
                                        let usage = &event.message.usage;
                                        input_tokens = usage.input_tokens
                                            + usage.cache_creation_input_tokens
                                            + usage.cache_read_input_tokens;
                                        debug!(
                                            model = %event.message.model,
                                            input_tokens,
                                            cache_write = usage.cache_creation_input_tokens,
                                            cache_read = usage.cache_read_input_tokens,
                                            "[StreamChat] Message started"
                                        );
                                    }
                                }
                                "content_block_start" => {
                                    if let Ok(event) = serde_json::from_str::<ContentBlockStartEvent>(data) {
                                        match event.content_block {
                                            ContentBlockStart::Text { .. } => {
                                                active_block = Some(ActiveBlock::Text);
                                            }
                                            ContentBlockStart::ToolUse { id, name } => {
                                                debug!(
                                                    tool = %name,
                                                    id = %id,
                                                    "[StreamChat] Tool use started"
                                                );
                                                active_block = Some(ActiveBlock::ToolUse {
                                                    id,
                                                    name,
                                                    input_json: String::new(),
                                                });
                                            }
                                        }
                                    }
                                }
                                "content_block_delta" => {
                                    if let Ok(event) = serde_json::from_str::<ContentBlockDeltaEvent>(data) {
                                        match event.delta {
                                            Delta::TextDelta { text } => {
                                                accumulated_text.push_str(&text);

                                                let _ = app_handle.emit(
                                                    "chat-token",
                                                    ChatTokenEvent {
                                                        session_id: session_id.clone(),
                                                        text,
                                                    },
                                                );
                                            }
                                            Delta::InputJsonDelta { partial_json } => {
                                                // Accumulate tool input JSON
                                                if let Some(ActiveBlock::ToolUse { ref mut input_json, .. }) = active_block {
                                                    input_json.push_str(&partial_json);
                                                }
                                            }
                                        }
                                    }
                                }
                                "content_block_stop" => {
                                    // Block completed - check if it was a tool use
                                    if let Some(ActiveBlock::ToolUse { id, name, input_json }) = active_block.take() {
                                        // Parse the accumulated JSON
                                        let input: serde_json::Value = serde_json::from_str(&input_json)
                                            .unwrap_or(serde_json::json!({}));

                                        debug!(
                                            tool = %name,
                                            input = ?input,
                                            "[StreamChat] Tool use block complete"
                                        );

                                        pending_tool_use = Some(ToolUseRequest { id, name, input });
                                    }
                                    active_block = None;
                                }
                                "message_delta" => {
                                    if let Ok(event) = serde_json::from_str::<MessageDeltaEvent>(data) {
                                        output_tokens = event.usage.output_tokens;
                                        stop_reason = event.delta.stop_reason;

                                        // Log if response was truncated
                                        if stop_reason.as_deref() == Some("max_tokens") {
                                            warn!(
                                                session_id = %session_id,
                                                output_tokens,
                                                "[StreamChat] Response truncated due to max_tokens limit"
                                            );
                                        }
                                    }
                                }
                                "message_stop" => {
                                    // Check if we have a pending tool use
                                    if let Some(tool_request) = pending_tool_use {
                                        info!(
                                            session_id = %session_id,
                                            tool = %tool_request.name,
                                            "[StreamChat] Returning for tool execution"
                                        );

                                        return Ok(StreamResult::ToolUse {
                                            text_so_far: accumulated_text,
                                            tool_request,
                                            input_tokens,
                                            output_tokens,
                                        });
                                    }

                                    info!(
                                        session_id = %session_id,
                                        input_tokens,
                                        output_tokens,
                                        text_len = accumulated_text.len(),
                                        "[StreamChat] Stream complete"
                                    );

                                    let _ = app_handle.emit(
                                        "chat-complete",
                                        ChatCompleteEvent {
                                            session_id: session_id.clone(),
                                            full_text: accumulated_text.clone(),
                                            input_tokens,
                                            output_tokens,
                                            stop_reason: stop_reason.clone(),
                                        },
                                    );

                                    return Ok(StreamResult::Complete {
                                        full_text: accumulated_text,
                                        input_tokens,
                                        output_tokens,
                                        stop_reason,
                                    });
                                }
                                "error" => {
                                    if let Ok(event) = serde_json::from_str::<ErrorEvent>(data) {
                                        let error_msg = format!(
                                            "{}: {}",
                                            event.error.error_type, event.error.message
                                        );

                                        warn!(session_id = %session_id, error = %error_msg, "[StreamChat] API error");

                                        let _ = app_handle.emit(
                                            "chat-error",
                                            ChatErrorEvent {
                                                session_id,
                                                error_type: event.error.error_type,
                                                message: event.error.message,
                                            },
                                        );

                                        return Err(Error::Api(error_msg));
                                    }
                                }
                                "ping" => {
                                    // Keepalive, ignore
                                }
                                _ => {
                                    debug!(event_type, "[StreamChat] Unknown event type");
                                }
                            }
                        }
                        current_event_type = None;
                    }
                }
                Ok(None) => {
                    // Stream ended unexpectedly
                    warn!(session_id = %session_id, "[StreamChat] Stream ended unexpectedly");

                    // If we have a pending tool use, return it
                    if let Some(tool_request) = pending_tool_use {
                        return Ok(StreamResult::ToolUse {
                            text_so_far: accumulated_text,
                            tool_request,
                            input_tokens,
                            output_tokens,
                        });
                    }

                    // If we have accumulated text, emit complete anyway
                    if !accumulated_text.is_empty() {
                        let _ = app_handle.emit(
                            "chat-complete",
                            ChatCompleteEvent {
                                session_id,
                                full_text: accumulated_text.clone(),
                                input_tokens,
                                output_tokens,
                                stop_reason: stop_reason.clone(),
                            },
                        );

                        return Ok(StreamResult::Complete {
                            full_text: accumulated_text,
                            input_tokens,
                            output_tokens,
                            stop_reason,
                        });
                    }

                    break;
                }
                Err(e) => {
                    let error_msg = format!("Stream read error: {}", e);
                    warn!(session_id = %session_id, error = %error_msg, "[StreamChat] Read error");

                    let _ = app_handle.emit(
                        "chat-error",
                        ChatErrorEvent {
                            session_id,
                            error_type: "stream_error".to_string(),
                            message: error_msg.clone(),
                        },
                    );

                    return Err(Error::Api(error_msg));
                }
            }
        }

        Ok(StreamResult::Cancelled)
    }

    /// Stream a chat conversation without tools (backwards compatible)
    pub async fn stream_chat(
        &self,
        system_prompt: &str,
        messages: Vec<ChatMessage>,
        tier: Option<AiTier>,
        app_handle: AppHandle,
        session_id: String,
        cancel_token: Arc<AtomicBool>,
    ) -> Result<()> {
        // Convert simple messages to API format
        let api_messages: Vec<ApiMessage> = messages
            .into_iter()
            .map(|m| ApiMessage {
                role: m.role,
                content: ApiContent::Text(m.content),
            })
            .collect();

        let result = self
            .stream_chat_with_tools(
                system_prompt,
                api_messages,
                None,
                tier,
                app_handle,
                session_id,
                cancel_token,
            )
            .await?;

        match result {
            StreamResult::Complete { .. } | StreamResult::Cancelled => Ok(()),
            StreamResult::ToolUse { .. } => {
                // Shouldn't happen without tools, but handle it
                Err(Error::Api("Unexpected tool use without tools enabled".into()))
            }
        }
    }
}
