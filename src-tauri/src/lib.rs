pub mod ai;
pub mod local_store;
pub mod strategy_convert;
pub mod notifications;
pub mod event_sink;
pub mod hub_stream;

// The trading core now lives in `wickd-core`. Re-export its modules at the
// crate root so existing `crate::oanda`, `crate::backtest`, `crate::config`,
// `crate::strategy`, `crate::models`, `crate::crypto`, `crate::error` paths
// throughout the desktop code keep resolving unchanged.
pub use wickd_core::{backtest, config, crypto, error, models, oanda, strategy};

pub use event_sink::TauriEventSink;
pub use ai::ClaudeClient;
pub use notifications::{focus_watcher_window_on_activation, send_test_notification, set_notifications_enabled};
pub use config::{Config, CompileTimeConfig};
pub use error::{Error, Result};
