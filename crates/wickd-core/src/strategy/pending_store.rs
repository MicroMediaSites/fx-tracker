//! Pending Matches Store
//!
//! Thread-safe storage for pattern matches that were emitted while the frontend
//! may not have been listening. Allows the frontend to fetch missed matches
//! when the Live Monitor window opens.

use std::sync::Mutex;
use once_cell::sync::Lazy;

/// Global store for pending pattern matches
/// Stores serialized JSON values for easy emission to frontend
static PENDING_MATCHES: Lazy<Mutex<Vec<serde_json::Value>>> = Lazy::new(|| Mutex::new(Vec::new()));

/// Maximum number of pending matches to store (prevents unbounded growth)
const MAX_PENDING_MATCHES: usize = 100;

/// Add a match to the pending store
pub fn add_pending_match(match_event: serde_json::Value) {
    if let Ok(mut store) = PENDING_MATCHES.lock() {
        store.push(match_event);
        // Keep only the most recent matches
        if store.len() > MAX_PENDING_MATCHES {
            let drain_count = store.len() - MAX_PENDING_MATCHES;
            store.drain(0..drain_count);
        }
    }
}

/// Get all pending matches
pub fn get_pending_matches() -> Vec<serde_json::Value> {
    PENDING_MATCHES.lock().map(|s| s.clone()).unwrap_or_default()
}

/// Clear all pending matches
pub fn clear_pending_matches() {
    if let Ok(mut store) = PENDING_MATCHES.lock() {
        store.clear();
    }
}

/// Remove a specific match by ID
pub fn remove_pending_match(match_id: &str) {
    if let Ok(mut store) = PENDING_MATCHES.lock() {
        store.retain(|m| {
            m.get("pattern_match")
                .and_then(|pm| pm.get("id"))
                .and_then(|id| id.as_str())
                .map(|id| id != match_id)
                .unwrap_or(true)
        });
    }
}
