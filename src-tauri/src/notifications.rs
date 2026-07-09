//! Desktop Notification Support
//!
//! Provides macOS-native notifications for pattern matches.
//!
//! When a notification is clicked, macOS brings the app to foreground and we
//! focus the Strategy Watcher window where the user can see pending matches.
//!
//! NOTE: We intentionally do NOT use `wait_for_click(true)` because it blocks
//! in FFI code, and when macOS bulk-clears notifications from the notification
//! center, it causes crashes that Rust's `catch_unwind` cannot intercept (BUG-019).

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager};
use tracing::{info, warn};

/// Global flag to enable/disable notifications
/// Controlled from frontend via set_notifications_enabled command
static NOTIFICATIONS_ENABLED: AtomicBool = AtomicBool::new(false);

/// Set whether notifications are enabled globally
pub fn set_notifications_enabled(enabled: bool) {
    NOTIFICATIONS_ENABLED.store(enabled, Ordering::SeqCst);
    info!("[Notification] Notifications {}", if enabled { "enabled" } else { "disabled" });
}

/// Check if notifications are enabled
pub fn are_notifications_enabled() -> bool {
    NOTIFICATIONS_ENABLED.load(Ordering::SeqCst)
}

/// Data attached to a pattern match notification.
/// Currently used for logging; click handling is done via app activation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationClickedPayload {
    /// The pattern match ID
    pub match_id: String,
    /// Instrument (e.g., "EUR_USD")
    pub instrument: String,
    /// Timeframe (e.g., "H4")
    pub timeframe: String,
    /// Strategy ID
    pub strategy_id: String,
    /// Strategy name for display
    pub strategy_name: String,
    /// Direction ("Long" or "Short")
    pub direction: String,
    /// Entry price as string
    pub entry_price: String,
    /// Stop loss price as string
    pub stop_loss: String,
    /// Take profit price as string
    pub take_profit: String,
    /// Pattern match timestamp (Unix ms)
    pub match_time: i64,
}

/// Send a pattern match notification.
///
/// On macOS, this uses `mac-notification-sys` to show a native notification.
/// When clicked, macOS brings the app to foreground. The app's activation
/// handler then focuses the Strategy Watcher window.
///
/// We do NOT use `wait_for_click(true)` because it causes crashes when
/// notifications are bulk-cleared from the notification center (BUG-019).
///
/// Respects the global NOTIFICATIONS_ENABLED flag - does nothing if disabled.
#[cfg(target_os = "macos")]
pub fn send_pattern_match_notification(_app_handle: AppHandle, payload: NotificationClickedPayload) {
    // Check if notifications are enabled
    if !are_notifications_enabled() {
        info!(
            "[Notification] Skipping notification for {} {} (notifications disabled)",
            payload.instrument, payload.timeframe
        );
        return;
    }

    use mac_notification_sys::{set_application, Notification};

    let title = payload.instrument.replace('_', "/");
    let body = payload.strategy_name.clone();

    info!(
        "[Notification] Sending notification for {} {} (match: {})",
        payload.instrument, payload.timeframe, payload.match_id
    );

    // Set application bundle - ignore "already set" error since that's fine
    if let Err(e) = set_application("com.openthink.wickd") {
        let err_str = e.to_string();
        if !err_str.contains("can only be set once") {
            warn!("[Notification] Failed to set application bundle: {}", e);
            return;
        }
    }

    // NOTE: Do NOT use main_button() - it causes the send() function to run an
    // NSRunLoop that busy-polls forever, consuming 100% CPU per notification.
    // See BUG-020 / debug-cpu-issue.md for details.
    let mut notification = Notification::new();
    let result = notification
        .title(&title)
        .subtitle("Pattern Match")
        .message(&body)
        .default_sound()
        .send();

    if let Err(e) = result {
        warn!("[Notification] Failed to send notification: {}", e);
    }
}

/// Fallback for non-macOS platforms - just logs that notifications aren't supported.
/// The frontend will still receive the pattern match and can show its own notification.
#[cfg(not(target_os = "macos"))]
pub fn send_pattern_match_notification(_app_handle: AppHandle, payload: NotificationClickedPayload) {
    info!(
        "[Notification] Platform doesn't support click handling, skipping native notification for {} {}",
        payload.instrument, payload.timeframe
    );
}

/// Send a backtest job completion notification.
///
/// On macOS, this uses `mac-notification-sys` to show a native notification.
#[cfg(target_os = "macos")]
pub fn send_job_completion_notification(
    _app_handle: AppHandle,
    strategy_name: &str,
    success: bool,
    pnl: Option<f64>,
    efficiency: Option<f64>,
    error: Option<&str>,
) {
    use mac_notification_sys::{set_application, Notification};

    // Set application bundle - ignore "already set" error since that's fine
    if let Err(e) = set_application("com.openthink.wickd") {
        let err_str = e.to_string();
        if !err_str.contains("can only be set once") {
            warn!("[Notification] Failed to set application bundle: {}", e);
            return;
        }
    }

    let (title, body) = if success {
        let pnl_str = pnl.map(|p| {
            if p >= 0.0 {
                format!("+${:.2}", p)
            } else {
                format!("-${:.2}", p.abs())
            }
        }).unwrap_or_default();

        let eff_str = efficiency.map(|e| format!("{:.0}% efficiency", e)).unwrap_or_default();

        (
            "Walk-Forward Complete".to_string(),
            format!("{}: {} {}", strategy_name, pnl_str, eff_str),
        )
    } else {
        (
            "Walk-Forward Failed".to_string(),
            format!("{}: {}", strategy_name, error.unwrap_or("Unknown error")),
        )
    };

    info!(
        "[Notification] Sending job completion notification for {}",
        strategy_name
    );

    // NOTE: Do NOT use main_button() - it causes the send() function to run an
    // NSRunLoop that busy-polls forever, consuming 100% CPU per notification.
    // See BUG-020 / debug-cpu-issue.md for details.
    let result = Notification::new()
        .title(&title)
        .message(&body)
        .default_sound()
        .send();

    if let Err(e) = result {
        warn!("[Notification] Failed to send notification: {}", e);
    }
}

/// Fallback for non-macOS platforms.
#[cfg(not(target_os = "macos"))]
pub fn send_job_completion_notification(
    _app_handle: AppHandle,
    strategy_name: &str,
    _success: bool,
    _pnl: Option<f64>,
    _efficiency: Option<f64>,
    _error: Option<&str>,
) {
    info!(
        "[Notification] Platform doesn't support native notifications for job completion: {}",
        strategy_name
    );
}

/// Send a test notification to verify notifications are working.
/// Sends synchronously - the send() call should return quickly.
#[cfg(target_os = "macos")]
pub fn send_test_notification() -> bool {
    use mac_notification_sys::{set_application, Notification};

    info!("[Notification] Sending test notification");

    // Set application bundle - ignore "already set" error since that's fine
    if let Err(e) = set_application("com.openthink.wickd") {
        let err_str = e.to_string();
        if !err_str.contains("can only be set once") {
            warn!("[Notification] Failed to set application bundle: {}", e);
            return false;
        }
        // Already set is fine, continue
    }

    let mut notification = Notification::new();
    let result = notification
        .title("wickd")
        .subtitle("Test Notification")
        .message("Notifications are working! You'll see alerts like this when patterns match.")
        .default_sound()
        .send();

    match result {
        Ok(_) => {
            info!("[Notification] Test notification sent successfully");
            true
        }
        Err(e) => {
            warn!("[Notification] Failed to send test notification: {}", e);
            false
        }
    }
}

/// Fallback for non-macOS platforms.
#[cfg(not(target_os = "macos"))]
pub fn send_test_notification() -> bool {
    info!("[Notification] Platform doesn't support native notifications");
    false
}

/// Focus the Strategy Watcher window when the app is activated (e.g., from notification click).
/// Falls back to the main window if watcher isn't open, or any available window.
pub fn focus_watcher_window_on_activation(app_handle: &AppHandle) {
    // Try to focus watcher window first
    if let Some(window) = app_handle.get_webview_window("watcher") {
        if window.set_focus().is_ok() {
            info!("[Notification] Focused watcher window on app activation");
            return;
        }
    }

    // Fall back to main window
    if let Some(window) = app_handle.get_webview_window("main") {
        if window.set_focus().is_ok() {
            info!("[Notification] Focused main window on app activation (watcher not open)");
            return;
        }
    }

    // Fall back to any available window
    for (label, window) in app_handle.webview_windows() {
        if window.set_focus().is_ok() {
            info!("[Notification] Focused {} window on app activation", label);
            return;
        }
    }

    warn!("[Notification] No windows available to focus on app activation");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_payload_serialization() {
        let payload = NotificationClickedPayload {
            match_id: "test-123".to_string(),
            instrument: "EUR_USD".to_string(),
            timeframe: "H4".to_string(),
            strategy_id: "strat-456".to_string(),
            strategy_name: "My Strategy".to_string(),
            direction: "Long".to_string(),
            entry_price: "1.12345".to_string(),
            stop_loss: "1.12000".to_string(),
            take_profit: "1.13000".to_string(),
            match_time: 1234567890000,
        };

        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("matchId"));
        assert!(json.contains("matchTime"));
        assert!(json.contains("EUR_USD"));
    }
}
