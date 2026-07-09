//! Chat commands for the AI terminal.
//!
//! AGT-650: the cloud AI proxy (queries-service `/ai/chat`) and its
//! Clerk-token auth were removed with the rest of the Zero/Clerk data path.
//! The terminal chat surface is retired until the local AI path lands (the
//! watcher-engine/AI rewire epics); `is_chat_enabled` returns `false` so the
//! frontend hides chat affordances, and the streaming commands return a clear
//! error instead of silently degrading.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use tauri::State;
use tokio::sync::Mutex;

use candlesight_lib::ai::{AiTier, ChatContext, ChatMessage};

// ============================================================================
// Shared State (managed through AppState extension)
// ============================================================================

/// Tracks chat session state. Retained (empty) so window/state wiring in
/// `main.rs` stays stable while the chat surface is retired.
pub struct ChatSessionState {
    pub last_request: Mutex<Instant>,
    pub cancel_tokens: Arc<Mutex<HashMap<String, (Arc<AtomicBool>, Instant)>>>,
}

impl Default for ChatSessionState {
    fn default() -> Self {
        Self {
            last_request: Mutex::new(Instant::now()),
            cancel_tokens: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

const CHAT_RETIRED_ERROR: &str =
    "AI chat is unavailable: the cloud AI proxy was retired with the Zero/Clerk removal \
     (AGT-650). A local AI path returns with the watcher-engine rewire.";

// ============================================================================
// Commands
// ============================================================================

/// Start a streaming chat session (retired — see module docs).
#[tauri::command]
pub async fn chat_stream(
    session_id: String,
    context: ChatContext,
    message: String,
    history: Vec<ChatMessage>,
    model: Option<String>,
    enable_tools: Option<bool>,
) -> Result<(), String> {
    // Keep the invoke signature the frontend uses; fail clearly.
    let _ = (session_id, context, message, history, model, enable_tools);
    Err(CHAT_RETIRED_ERROR.to_string())
}

/// Cancel an ongoing chat stream. No streams can start while chat is
/// retired, so there is never anything to cancel.
#[tauri::command]
pub async fn chat_cancel(
    session_id: String,
    chat_state: State<'_, ChatSessionState>,
) -> Result<(), String> {
    let tokens = chat_state.cancel_tokens.lock().await;
    let _ = tokens.get(&session_id);
    Ok(())
}

/// Check if AI chat is available. Always `false` while the chat surface is
/// retired (AGT-650) — the frontend uses this to hide chat affordances.
#[tauri::command]
pub async fn is_chat_enabled() -> Result<bool, String> {
    Ok(false)
}

/// Input for chat compaction (kept for invoke-arg compatibility).
#[derive(serde::Deserialize)]
pub struct CompactionInput {
    /// The oldest uncompacted message to be merged
    pub oldest_message: ChatMessage,
    /// The existing compaction content (if any)
    pub existing_compaction: Option<String>,
}

/// Create a chat compaction (retired — see module docs).
#[tauri::command]
pub async fn create_chat_compaction(input: CompactionInput) -> Result<String, String> {
    let _ = input;
    Err(CHAT_RETIRED_ERROR.to_string())
}

/// Verify which model actually answers for a requested AI tier (used by the
/// debug overlay). Lived in the deleted analysis command module before
/// AGT-652 slimmed the app surface.
#[tauri::command]
pub async fn check_ai_model(
    model: String,
    state: State<'_, crate::AppState>,
) -> Result<String, String> {
    let client = state.claude.as_ref()
        .ok_or_else(|| "AI not enabled".to_string())?;

    let tier = AiTier::from_str(&model);
    let requested = tier.model();

    let actual = client.check_model(tier)
        .await
        .map_err(|e| e.to_string())?;

    Ok(format!("Requested: {} -> Actual: {}", requested, actual))
}

/// Attempt to recover a strategy from a parsing error using AI
///
/// When a strategy fails to parse (e.g., invalid enum value, missing field),
/// this command uses Haiku to analyze the error and suggest minimal fixes.
/// This is NOT a strategy builder - it only repairs broken JSON. (Moved from
/// the deleted analysis command module; the backtest window still uses it.)
#[tauri::command]
pub async fn recover_strategy_error(
    state: State<'_, crate::AppState>,
    error_message: String,
    strategy_json: String,
) -> Result<candlesight_lib::ai::RecoveryResult, String> {
    let claude = state.claude.as_ref()
        .ok_or_else(|| "AI features not available".to_string())?;

    candlesight_lib::ai::strategy_recovery::recover_strategy_error(
        claude,
        &error_message,
        &strategy_json,
    ).await
}
