//! The app as a reader of the AI market-awareness feed (`wickd feed tick`).
//!
//! The producer is a headless launchd one-shot (`com.openthink.wickd-feed`)
//! that appends to `~/.wickd/feed.ndjson`; the app only reads and renders —
//! read-only and offline, same contract as the calendar and daemon commands
//! (the offline-boot e2e specs stay green).

use serde::Deserialize;
use tokio::io::AsyncWriteExt;

use wickd_core::feed::{self, FeedItem};

use super::daemon::find_wickd_binary;

/// One prior turn of the drawer's ask transcript, threaded back to `wickd feed
/// ask` so a follow-up sees what was already discussed.
#[derive(Debug, Deserialize)]
pub struct AskTurn {
    pub role: String,
    pub text: String,
}

/// Feed items, newest first, capped at `limit` (default 100).
#[tauri::command]
pub async fn feed_list(limit: Option<usize>) -> Result<Vec<FeedItem>, String> {
    let path = feed::feed_path().map_err(|e| e.to_string())?;
    let mut items = feed::list_at(path).map_err(|e| e.to_string())?;
    items.reverse();
    items.truncate(limit.unwrap_or(100));
    Ok(items)
}

/// Ask a follow-up question about the feed. Shells out to `wickd feed ask` —
/// everything AI (claude spawn, subscription auth via the config's
/// claude_config_dir, prompt guardrails) stays in the CLI; the app only
/// relays the question and renders the answer. User-triggered, never on the
/// boot path.
#[tauri::command]
pub async fn feed_ask(question: String, history: Option<Vec<AskTurn>>) -> Result<String, String> {
    let question = question.trim().to_string();
    if question.is_empty() {
        return Err("question is empty".to_string());
    }
    if question.chars().count() > 2000 {
        return Err("question is too long (2000 chars max)".to_string());
    }
    let wickd = find_wickd_binary()
        .ok_or_else(|| "wickd CLI not found — install it (cargo install) to use the feed".to_string())?;

    // Thread the prior transcript through the CLI's `--history -` stdin path
    // (avoids argv length limits). Only user/assistant turns are relayed; the
    // CLI fences + neutralizes them, same trust boundary as every ask input.
    let history: Vec<AskTurn> = history
        .unwrap_or_default()
        .into_iter()
        .filter(|t| t.role == "user" || t.role == "assistant")
        .collect();
    let history_json = serde_json::to_string(
        &history
            .iter()
            .map(|t| serde_json::json!({ "role": t.role, "text": t.text }))
            .collect::<Vec<_>>(),
    )
    .map_err(|e| format!("serializing ask history: {e}"))?;

    let mut child = tokio::process::Command::new(&wickd)
        .args(["feed", "ask", &question, "--history", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("running wickd feed ask: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(history_json.as_bytes())
            .await
            .map_err(|e| format!("sending ask history: {e}"))?;
        // Drop closes stdin so the CLI's read_to_string returns.
    }

    let output = tokio::time::timeout(std::time::Duration::from_secs(150), child.wait_with_output())
        .await
        .map_err(|_| "the answer timed out".to_string())?
        .map_err(|e| format!("running wickd feed ask: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .map_err(|_| format!("unexpected wickd output: {}", stdout.chars().take(200).collect::<String>()))?;
    if let Some(err) = value.get("error") {
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
        return Err(msg.to_string());
    }
    value
        .get("answer")
        .and_then(|a| a.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "wickd returned no answer".to_string())
}
