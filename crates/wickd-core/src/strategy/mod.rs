//! Strategy Module
//!
//! Provides live strategy execution capabilities by connecting:
//! - Candle data sources (polling or streaming)
//! - Indicator calculations
//! - Rule evaluation
//! - Pattern match detection (when user-defined conditions are met)
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                     Strategy Watcher Service                     │
//! ├─────────────────────────────────────────────────────────────────┤
//! │                                                                  │
//! │  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐       │
//! │  │ CandleSource │───▶│IndicatorEng │───▶│ RulesEngine  │       │
//! │  │              │    │              │    │              │       │
//! │  │ Poll OANDA   │    │ Updates on   │    │ Evaluates    │       │
//! │  │ /candles API │    │ new candle   │    │ entry/exit   │       │
//! │  └──────────────┘    └──────────────┘    └──────────────┘       │
//! │         ▲                                        │               │
//! │         │                                        ▼               │
//! │  ┌──────────────┐    ┌──────────────┐   ┌──────────────┐        │
//! │  │ OANDA REST   │    │ Position     │   │PatternMatch  │        │
//! │  │ API          │    │ Checker      │   │   Emitter    │        │
//! │  └──────────────┘    └──────────────┘   └──────────────┘        │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Execution Modes
//!
//! 1. **AlertOnly** - Emit pattern matches to frontend, user decides to trade
//! 2. **ConfirmExecute** - Emit matches, user confirms, app places order
//! 3. **AutoExecute** - Automatic execution (future feature)
//!
//! # Usage
//!
//! ```rust,ignore
//! use fx_tracker::strategy::{StrategyWatcher, WatcherConfig, ExecutionMode};
//!
//! // Create watcher from strategy config
//! let config = WatcherConfig {
//!     user_id: "user123".to_string(),
//!     instrument: "EUR_USD".to_string(),
//!     timeframe: Granularity::H1,
//!     ..Default::default()
//! };
//!
//! let mut watcher = StrategyWatcher::new(
//!     "config_id".to_string(),
//!     config,
//!     strategy_definition,
//!     oanda_client,
//!     ExecutionMode::AlertOnly,
//! )?;
//!
//! // Start in background task
//! watcher.start(app_handle).await?;
//!
//! // Stop when done
//! watcher.stop();
//! ```

mod candle_boundary;
mod candle_source;
mod multi_watcher;
mod pattern_match;
mod pending_store;
mod tick_stream_source;
mod watch_state;
mod watcher;

pub use candle_boundary::{CandleBoundaryDetector, CandleBoundaryService, CandleCloseEvent};
pub use candle_source::{CandleSource, OandaPollingSource, StreamingCandleSource};
pub use tick_stream_source::{Tick, TickCandleAggregator, TickStreamSource};
pub use multi_watcher::{MultiInstrumentWatcher, MultiWatcherHandle, WatcherCommand};
pub use pattern_match::{
    // New names
    MatchStatus, MatchType, PatternMatch, PatternMatchEvent, MatchStatusUpdateEvent,
    // Compatibility aliases
    SignalStatus, SignalType, StrategySignal, StrategySignalEvent, SignalStatusUpdateEvent,
    // Other types
    StrategyErrorEvent, StrategyStatusEvent, WatcherStatus, WatcherTickEvent,
    IndicatorSnapshot,
};
pub use watch_state::WatchStateStore;
pub use watcher::{ExecutionMode, StrategyWatcher, WatcherConfig, WatcherInfo};
pub use pending_store::{add_pending_match, get_pending_matches, clear_pending_matches, remove_pending_match};
