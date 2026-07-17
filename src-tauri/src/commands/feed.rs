//! The app as a reader of the AI market-awareness feed (`wickd feed tick`).
//!
//! The producer is a headless launchd one-shot (`com.openthink.wickd-feed`)
//! that appends to `~/.wickd/feed.ndjson`; the app only reads and renders —
//! read-only and offline, same contract as the calendar and daemon commands
//! (the offline-boot e2e specs stay green).

use wickd_core::feed::{self, FeedItem};

/// Feed items, newest first, capped at `limit` (default 100).
#[tauri::command]
pub async fn feed_list(limit: Option<usize>) -> Result<Vec<FeedItem>, String> {
    let path = feed::feed_path().map_err(|e| e.to_string())?;
    let mut items = feed::list_at(path).map_err(|e| e.to_string())?;
    items.reverse();
    items.truncate(limit.unwrap_or(100));
    Ok(items)
}
