//! Strategy Watcher
//!
//! Orchestrates live strategy execution by connecting candle data,
//! indicator calculations, and rule evaluation to generate trading signals.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::time::sleep;
use tracing::{info, warn};

use crate::backtest::mtf::{self, MtfCandleStore};
use crate::backtest::rules_engine::{PositionDirection, RulesEngine, RulesSignal, SRZone, StrategyDefinition};
use crate::backtest::scripted_strategy::ScriptedStrategy;
use crate::backtest::strategy::{Signal, ExtendedSignal, Strategy};
use crate::error::Result;
use crate::models::Candle;
use crate::oanda::client::OandaClient;
use crate::oanda::endpoints::{self, get_account, get_open_positions, Granularity};

use super::candle_source::{CandleSource, OandaPollingSource};
use super::pattern_match::{
    IndicatorSnapshot, StrategyErrorEvent, StrategyStatusEvent, WatcherStatus, WatcherTickEvent,
    // Use aliases for backwards compat during migration
    PatternMatch as StrategySignal, PatternMatchEvent as StrategySignalEvent,
    MatchType as SignalType, MatchStatus as SignalStatus,
    MatchStatusUpdateEvent as SignalStatusUpdateEvent,
};
use crate::event_sink::EventSink;

/// Default number of candles before a signal expires
const DEFAULT_SIGNAL_TTL_CANDLES: u32 = 3;

/// Base backoff duration for network errors (seconds)
const BASE_BACKOFF_SECS: u64 = 5;

/// Maximum backoff duration (5 minutes)
const MAX_BACKOFF_SECS: u64 = 300;

// =============================================================================
// StrategyExecutor — wraps either a RulesEngine or ScriptedStrategy
// =============================================================================

/// Convert a scripted strategy's ExtendedSignal into the RulesSignal that the
/// watcher pipeline expects.
fn signal_to_rules_signal(ext: ExtendedSignal, _position_direction: Option<PositionDirection>) -> RulesSignal {
    match ext.signal {
        Signal::Buy => RulesSignal::Entry {
            direction: PositionDirection::Long,
            stop_loss: ext.stop_loss,
            take_profit: ext.take_profit,
            triggered_rule_id: ext.entry_rule_id,
            triggered_rule_name: ext.entry_rule_name,
            pending_order: ext.pending_order,
        },
        Signal::Sell => RulesSignal::Entry {
            direction: PositionDirection::Short,
            stop_loss: ext.stop_loss,
            take_profit: ext.take_profit,
            triggered_rule_id: ext.entry_rule_id,
            triggered_rule_name: ext.entry_rule_name,
            pending_order: ext.pending_order,
        },
        Signal::ClosePosition => RulesSignal::Exit {
            reason: ext.exit_reason.unwrap_or_else(|| "Script exit".to_string()),
            close_percent: 1.0,
        },
        Signal::Hold => RulesSignal::Hold,
    }
}

/// Executor that wraps either a rules-based or scripted strategy for the
/// live watcher system.
pub(crate) enum StrategyExecutor {
    Rules(RulesEngine),
    Scripted(ScriptedStrategy),
}

impl StrategyExecutor {
    /// Process a candle in live mode, returning a `RulesSignal`.
    ///
    /// For rules-based strategies this delegates to `on_candle_live`.
    /// For scripted strategies we call `on_candle_extended` and convert
    /// the resulting `ExtendedSignal` into a `RulesSignal`.
    pub fn on_candle_live(&mut self, candle: &Candle, position_direction: Option<PositionDirection>) -> RulesSignal {
        match self {
            Self::Rules(engine) => engine.on_candle_live(candle, position_direction),
            Self::Scripted(strategy) => {
                let ext = strategy.on_candle_extended(candle);
                signal_to_rules_signal(ext, position_direction)
            }
        }
    }

    /// Warm up with a historical candle. Rules: indicators only (no rule
    /// evaluation). Scripted: the full `on_candle()` path with the signal
    /// discarded, so script-local state machines are warm after a restart
    /// (issue #9).
    pub fn warmup_candle(&mut self, candle: &Candle) {
        match self {
            Self::Rules(engine) => engine.warmup_candle(candle),
            Self::Scripted(strategy) => strategy.warmup_candle(candle),
        }
    }

    /// Get a snapshot of current indicator values for auto-notes.
    pub fn get_indicator_snapshot(&self) -> HashMap<String, HashMap<String, String>> {
        match self {
            Self::Rules(engine) => engine.get_indicator_snapshot(),
            Self::Scripted(strategy) => strategy.get_indicator_snapshot(),
        }
    }

    /// Set S/R zones (rules-engine only, no-op for scripted).
    pub fn set_sr_zones(&mut self, zones: Vec<SRZone>) {
        if let Self::Rules(engine) = self {
            engine.set_sr_zones(zones);
        }
    }

    /// Reclassify indicators whose explicit timeframe matches the chart
    /// timeframe (rules-engine only, no-op for scripted).
    #[allow(dead_code)]
    pub fn set_primary_granularity(&mut self, timeframe: &str) {
        if let Self::Rules(engine) = self {
            engine.set_primary_granularity(timeframe);
        }
    }

    /// Returns `Some(reason)` exactly once, on the first candle after a scripted
    /// strategy's `on_candle()` consecutive-error abort threshold trips — so callers
    /// can emit a `strategy_error` event that looks nothing like the `Hold` a healthy
    /// script legitimately returns most of the time (AGT-606). Always `None` for
    /// rules-based strategies, which report failures through `process_tick`'s
    /// `Result` instead.
    pub fn take_health_event(&mut self) -> Option<String> {
        match self {
            Self::Rules(_) => None,
            Self::Scripted(strategy) => {
                if strategy.take_abort_event() {
                    Some(strategy.abort_reason())
                } else {
                    None
                }
            }
        }
    }

    /// Set pip value for the instrument.
    #[allow(dead_code)]
    pub fn set_pip_value_for_instrument(&mut self, instrument: &str) {
        match self {
            Self::Rules(engine) => engine.set_pip_value_for_instrument(instrument),
            Self::Scripted(strategy) => strategy.set_pip_value_for_instrument(instrument),
        }
    }

    /// Set the MTF candle store (rules-engine only, no-op for scripted).
    pub fn set_mtf_candle_store(&mut self, store: MtfCandleStore) {
        if let Self::Rules(engine) = self {
            engine.set_mtf_candle_store(store);
        }
    }

    /// Append an HTF candle (rules-engine only, no-op for scripted).
    pub fn append_htf_candle(&mut self, tf: &str, candle: Candle) {
        if let Self::Rules(engine) = self {
            engine.append_htf_candle(tf, candle);
        }
    }

    /// Calculate position size (rules-engine only).
    /// For scripted strategies, position sizing comes from the script itself
    /// via stop_loss/take_profit in the signal, so we return None.
    pub fn calculate_position_size(
        &self,
        balance: rust_decimal::Decimal,
        entry_price: rust_decimal::Decimal,
        stop_loss: rust_decimal::Decimal,
        direction: PositionDirection,
    ) -> Option<rust_decimal::Decimal> {
        match self {
            Self::Rules(engine) => engine.calculate_position_size(balance, entry_price, stop_loss, direction),
            Self::Scripted(_) => None,
        }
    }
}

/// Configuration for a strategy watcher
#[derive(Debug, Clone)]
pub struct WatcherConfig {
    /// User ID who owns this watcher
    pub user_id: String,
    /// Strategy name for display purposes
    pub strategy_name: String,
    /// Instrument to watch (e.g., "EUR_USD")
    pub instrument: String,
    /// Candle timeframe
    pub timeframe: Granularity,
    /// Override poll interval (uses default if None)
    pub poll_interval: Option<Duration>,
    /// Number of candles to warm up indicators
    pub warmup_candles: u32,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            user_id: String::new(),
            strategy_name: String::new(),
            instrument: "EUR_USD".to_string(),
            timeframe: Granularity::H1,
            poll_interval: None,
            warmup_candles: 100,
        }
    }
}

/// Execution mode for the strategy watcher
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    /// Only emit signals, no automatic execution
    SignalOnly,
    /// Emit signals and wait for user confirmation before executing
    ConfirmExecute,
    /// Automatically execute signals (future feature)
    AutoExecute,
}

impl std::str::FromStr for ExecutionMode {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "signal_only" => Ok(ExecutionMode::SignalOnly),
            "confirm_execute" => Ok(ExecutionMode::ConfirmExecute),
            "auto_execute" => Ok(ExecutionMode::AutoExecute),
            _ => Err(format!("Invalid execution mode: {}", s)),
        }
    }
}

/// Tracks a pending signal for invalidation purposes
#[derive(Debug, Clone)]
struct PendingSignal {
    /// The signal that was emitted
    signal: StrategySignal,
    /// Number of candles since this signal was generated
    candles_since: u32,
}

/// Strategy watcher that monitors the market and generates trading signals
pub struct StrategyWatcher {
    /// Unique ID for this watcher (matches strategy_config ID)
    config_id: String,
    /// Configuration
    config: WatcherConfig,
    /// Candle data source
    candle_source: Box<dyn CandleSource>,
    /// Strategy executor (rules-based or scripted)
    executor: StrategyExecutor,
    /// OANDA client for position checks
    oanda_client: OandaClient,
    /// Execution mode (for future auto-execute feature)
    #[allow(dead_code)]
    mode: ExecutionMode,
    /// External stop signal (shared with the command handler)
    external_stop: Option<Arc<AtomicBool>>,
    /// Internal running flag
    running: AtomicBool,
    /// Flag indicating watcher is initialized
    initialized: AtomicBool,
    /// Pending signals that haven't been executed yet
    pending_signals: Vec<PendingSignal>,
    /// Signal TTL in candles (signals expire after this many candles)
    signal_ttl_candles: u32,
    /// Consecutive error count for exponential backoff
    consecutive_errors: u32,
    /// HTF timeframes required by this strategy (empty if single-timeframe)
    htf_timeframes: HashSet<String>,
    /// Last time HTF candles were refreshed (for rate-limiting live fetches)
    last_htf_refresh: Option<Instant>,
}

impl StrategyWatcher {
    /// Create a new strategy watcher with OANDA candle source.
    ///
    /// Routes to the correct executor based on `strategy.strategy_type`:
    /// - `"rules"` (default) -> `StrategyExecutor::Rules(RulesEngine)`
    /// - `"scripted"` -> `StrategyExecutor::Scripted(ScriptedStrategy)`
    pub fn new(
        config_id: String,
        config: WatcherConfig,
        strategy: StrategyDefinition,
        oanda_client: OandaClient,
        mode: ExecutionMode,
    ) -> Result<Self> {
        // Create the candle source
        let candle_source = Box::new(OandaPollingSource::new(
            oanda_client.clone(),
            config.instrument.clone(),
            config.timeframe,
        ));

        Self::with_candle_source(config_id, config, strategy, oanda_client, candle_source, mode)
    }

    /// Create a new strategy watcher with a custom candle source
    ///
    /// This is useful for testing with mock data or using alternative data sources.
    pub fn with_candle_source(
        config_id: String,
        config: WatcherConfig,
        strategy: StrategyDefinition,
        oanda_client: OandaClient,
        candle_source: Box<dyn CandleSource>,
        mode: ExecutionMode,
    ) -> Result<Self> {
        // Build the executor based on strategy type
        let (executor, htf_timeframes) = Self::create_executor(&config_id, &config, &strategy)?;

        Ok(Self {
            config_id,
            config,
            candle_source,
            executor,
            oanda_client,
            mode,
            external_stop: None,
            running: AtomicBool::new(false),
            initialized: AtomicBool::new(false),
            pending_signals: Vec::new(),
            signal_ttl_candles: DEFAULT_SIGNAL_TTL_CANDLES,
            consecutive_errors: 0,
            htf_timeframes,
            last_htf_refresh: None,
        })
    }

    /// Build a StrategyExecutor from a StrategyDefinition, handling routing
    /// between rules-based and scripted strategies.
    fn create_executor(
        config_id: &str,
        config: &WatcherConfig,
        strategy: &StrategyDefinition,
    ) -> Result<(StrategyExecutor, HashSet<String>)> {
        match strategy.strategy_type.as_str() {
            "scripted" => {
                let script = strategy.script_content.as_deref()
                    .ok_or_else(|| crate::error::Error::Strategy(
                        "Scripted strategy missing script_content".to_string()
                    ))?;

                let mut scripted = ScriptedStrategy::from_script(script, &strategy.name)
                    .map_err(|e| crate::error::Error::Strategy(e))?;

                scripted.set_pip_value_for_instrument(&config.instrument);

                info!(
                    "[Watcher {}] Created scripted strategy executor for '{}'",
                    config_id, strategy.name
                );

                // Scripted strategies don't support MTF, so no HTF timeframes
                Ok((StrategyExecutor::Scripted(scripted), HashSet::new()))
            }
            _ => {
                // Default: rules-based strategy
                // Extract HTF timeframes before moving strategy into the rules engine
                let htf_timeframes = mtf::extract_htf_timeframes(strategy, &config.timeframe.to_string());
                if !htf_timeframes.is_empty() {
                    info!(
                        "[Watcher {}] Strategy requires {} HTF timeframe(s): {:?}",
                        config_id, htf_timeframes.len(), htf_timeframes
                    );
                }

                let mut rules_engine = RulesEngine::new(strategy.clone())
                    .map_err(|e| crate::error::Error::Strategy(e))?;

                // Set pip value for the instrument (important for JPY pairs, gold, silver, indices)
                rules_engine.set_pip_value_for_instrument(&config.instrument);

                // Reclassify indicators whose explicit timeframe matches the chart timeframe
                rules_engine.set_primary_granularity(&config.timeframe.to_string());

                info!(
                    "[Watcher {}] Created rules-based strategy executor for '{}'",
                    config_id, strategy.name
                );

                Ok((StrategyExecutor::Rules(rules_engine), htf_timeframes))
            }
        }
    }

    /// Set an external stop signal for coordinating shutdown
    pub fn set_stop_signal(&mut self, stop_signal: Arc<AtomicBool>) {
        self.external_stop = Some(stop_signal);
    }

    /// Set S/R zones for zone-based trigger evaluation (rules-engine only)
    pub fn set_sr_zones(&mut self, zones: Vec<SRZone>) {
        self.executor.set_sr_zones(zones);
    }

    /// Set S/R zones from JSON string (rules-engine only)
    pub fn set_sr_zones_from_json(&mut self, json: &str) -> std::result::Result<(), String> {
        let zones: Vec<SRZone> = serde_json::from_str(json)
            .map_err(|e| format!("Failed to parse S/R zones JSON: {}", e))?;
        self.set_sr_zones(zones);
        Ok(())
    }

    /// Check if we should stop (either from internal or external signal)
    fn should_stop(&self) -> bool {
        if !self.running.load(Ordering::SeqCst) {
            return true;
        }
        if let Some(ref external) = self.external_stop {
            if external.load(Ordering::SeqCst) {
                return true;
            }
        }
        false
    }

    /// Start the watcher's main loop
    ///
    /// This runs in a background task and emits signals via the event sink.
    pub async fn start(&mut self, sink: Arc<dyn EventSink>) -> Result<()> {
        let sink = sink.as_ref();
        // Use compare_exchange to atomically check and set running state
        if self.running.compare_exchange(
            false,
            true,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ).is_err() {
            // Already running
            return Ok(());
        }

        // Emit started status
        self.emit_status(sink, WatcherStatus::Running, None);

        // Initialize indicators with historical data (with retry)
        if !self.initialized.load(Ordering::SeqCst) {
            if let Err(e) = self.warmup_with_retry(sink).await {
                self.emit_error(sink, "warmup_failed", &e.to_string());
                self.running.store(false, Ordering::SeqCst);
                self.emit_status(sink, WatcherStatus::Error, Some(e.to_string()));
                return Err(e);
            }
            self.initialized.store(true, Ordering::SeqCst);

            // Immediately evaluate the current market state after warmup
            // This gives an instant signal check instead of waiting for the next candle
            info!("[Watcher {}] Performing initial evaluation on current candle", self.config_id);
            if let Err(e) = self.evaluate_current_state(sink).await {
                // Non-fatal - just log and continue to normal polling
                warn!("[Watcher {}] Initial evaluation failed: {}", self.config_id, e);
            }
        }

        // Get poll interval
        let poll_interval = self.config.poll_interval
            .unwrap_or_else(|| self.candle_source.poll_interval());

        // Main loop
        while !self.should_stop() {
            match self.process_tick(sink).await {
                Ok(()) => {
                    // Reset consecutive errors on success
                    if self.consecutive_errors > 0 {
                        info!(
                            "[Watcher {}] Recovered after {} consecutive errors",
                            self.config_id, self.consecutive_errors
                        );
                        self.consecutive_errors = 0;
                        // Re-emit running status to clear any error state in UI
                        self.emit_status(sink, WatcherStatus::Running, None);
                    }
                }
                Err(e) => {
                    self.consecutive_errors += 1;
                    let error_msg = e.to_string();

                    // Check if this is a transient network error
                    if Self::is_transient_error(&error_msg) {
                        let backoff = self.calculate_backoff();
                        warn!(
                            "[Watcher {}] Transient error (attempt {}): {}. Backing off for {}s",
                            self.config_id, self.consecutive_errors, error_msg, backoff.as_secs()
                        );

                        // Only emit error event after several consecutive failures
                        if self.consecutive_errors >= 3 {
                            self.emit_error(sink, "transient_error", &format!(
                                "{} (retrying in {}s, attempt {})",
                                error_msg, backoff.as_secs(), self.consecutive_errors
                            ));
                        }

                        // Sleep for backoff duration instead of normal poll interval
                        sleep(backoff).await;
                        continue;
                    } else {
                        // Non-transient error - log and continue with normal interval
                        self.emit_error(sink, "tick_error", &error_msg);
                    }
                }
            }

            sleep(poll_interval).await;
        }

        // Clean up
        self.running.store(false, Ordering::SeqCst);

        self.emit_status(sink, WatcherStatus::Stopped, None);
        Ok(())
    }

    /// Warmup indicators with retry logic for network errors
    async fn warmup_with_retry(&mut self, sink: &dyn EventSink) -> Result<()> {
        let max_attempts = 5;
        let mut attempt = 0;

        loop {
            attempt += 1;
            match self.warmup_indicators().await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    let error_msg = e.to_string();
                    if attempt >= max_attempts {
                        return Err(e);
                    }
                    if Self::is_transient_error(&error_msg) {
                        let backoff = Duration::from_secs(BASE_BACKOFF_SECS * (1 << attempt.min(5)));
                        warn!(
                            "[Watcher {}] Warmup failed (attempt {}/{}): {}. Retrying in {}s",
                            self.config_id, attempt, max_attempts, error_msg, backoff.as_secs()
                        );
                        self.emit_error(sink, "warmup_retry", &format!(
                            "Initialization failed: {}. Retrying ({}/{})",
                            error_msg, attempt, max_attempts
                        ));
                        sleep(backoff).await;
                    } else {
                        // Non-transient error - don't retry
                        return Err(e);
                    }
                }
            }
        }
    }

    /// Check if an error is transient (network issues) and should be retried
    fn is_transient_error(error_msg: &str) -> bool {
        let transient_patterns = [
            "502 Bad Gateway",
            "503 Service",
            "504 Gateway",
            "connection refused",
            "Connection refused",
            "connection reset",
            "Connection reset",
            "timed out",
            "timeout",
            "Timeout",
            "temporarily unavailable",
            "Too Many Requests",
            "rate limit",
            "ECONNRESET",
            "ETIMEDOUT",
            "ENOTFOUND",
            "network",
            "Network",
            "socket",
            "Socket",
        ];

        transient_patterns.iter().any(|pattern| error_msg.contains(pattern))
    }

    /// Calculate exponential backoff duration based on consecutive errors
    fn calculate_backoff(&self) -> Duration {
        // Exponential backoff: base * 2^(errors-1), capped at max
        let exponent = (self.consecutive_errors.saturating_sub(1)).min(6);
        let backoff_secs = BASE_BACKOFF_SECS * (1 << exponent);
        Duration::from_secs(backoff_secs.min(MAX_BACKOFF_SECS))
    }

    /// Stop the watcher
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Check if the watcher is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Get the config ID
    pub fn config_id(&self) -> &str {
        &self.config_id
    }

    /// Get the instrument
    pub fn instrument(&self) -> &str {
        &self.config.instrument
    }

    /// Get the timeframe
    pub fn timeframe(&self) -> Granularity {
        self.config.timeframe
    }

    /// Evaluate the current market state immediately (used on startup)
    ///
    /// This fetches the most recent complete candle and evaluates rules against it,
    /// giving an immediate signal check instead of waiting for the next candle close.
    async fn evaluate_current_state(&mut self, sink: &dyn EventSink) -> Result<()> {
        // Fetch the most recent candles (2 to ensure we get a complete one)
        let candles = self.candle_source.get_candles(2).await?;

        // Get the most recent complete candle
        let candle = candles
            .iter()
            .filter(|c| c.complete)
            .max_by_key(|c| c.time)
            .ok_or_else(|| crate::error::Error::Strategy("No complete candle available".to_string()))?;

        info!(
            "[Watcher {}] Initial evaluation on candle: time={}, C={}",
            self.config_id, candle.time, candle.mid.close
        );

        // Check current position state
        let position_direction = self.check_open_position().await?;

        // Evaluate strategy on the current candle
        let signal = self.executor.on_candle_live(candle, position_direction);

        // A scripted strategy that just hit its consecutive-error abort threshold
        // reports it here — surface it as a distinct health event, not a silent Hold.
        if let Some(reason) = self.executor.take_health_event() {
            self.emit_error(sink, "script_aborted", &reason);
        }

        // Capture indicator snapshot for auto-notes
        let indicator_snapshot = self.executor.get_indicator_snapshot();

        // Filter and emit signal if applicable
        if let Some(strategy_signal) = self.filter_signal(signal, position_direction, candle, Some(indicator_snapshot)).await {
            info!(
                "[Watcher {}] Initial signal generated: {:?} {:?}",
                self.config_id, strategy_signal.match_type, strategy_signal.direction
            );

            // Track as pending signal
            self.pending_signals.push(PendingSignal {
                signal: strategy_signal.clone(),
                candles_since: 0,
            });

            self.emit_signal(sink, strategy_signal);
        } else {
            info!("[Watcher {}] No signal on initial evaluation (Hold)", self.config_id);
        }

        Ok(())
    }

    /// Warm up indicators with historical candles
    async fn warmup_indicators(&mut self) -> Result<()> {
        let candles = self.candle_source.get_candles(self.config.warmup_candles).await?;

        info!(
            "[Watcher {}] Warming up with {} candles, last candle time: {:?}",
            self.config_id,
            candles.len(),
            candles.last().map(|c| c.time)
        );

        // Fetch and set HTF candles before warmup so the RulesEngine can
        // advance HTF indicator engines during the warmup pass
        if !self.htf_timeframes.is_empty() {
            self.fetch_and_set_htf_candles(&candles).await?;
        }

        for candle in &candles {
            // Warm up indicators and price history only - don't evaluate rules
            // This prevents spurious signals from historical data
            self.executor.warmup_candle(candle);
        }

        // Prime the candle source by fetching latest candle
        // This ensures we don't re-process a candle that was already in warmup
        let primed = self.candle_source.get_latest_candle().await?;
        info!(
            "[Watcher {}] Primed candle source, got candle: {:?}",
            self.config_id,
            primed.map(|c| (c.time, c.mid.close))
        );

        Ok(())
    }

    /// Fetch HTF candles covering the warmup period and set them on the rules engine
    async fn fetch_and_set_htf_candles(&mut self, warmup_candles: &[crate::models::Candle]) -> Result<()> {
        use std::str::FromStr;

        // Determine the time range from warmup candles
        let from_time = warmup_candles.first()
            .map(|c| c.time.to_rfc3339())
            .ok_or_else(|| crate::error::Error::Strategy("No warmup candles to derive HTF range".to_string()))?;
        // Use a generous "to" that extends past the last candle to catch the latest HTF candle
        let to_time = chrono::Utc::now().to_rfc3339();

        let mut mtf_store = MtfCandleStore::new();

        for tf in &self.htf_timeframes {
            let htf_gran = Granularity::from_str(tf)
                .map_err(|e| crate::error::Error::Strategy(format!("Invalid HTF granularity '{}': {}", tf, e)))?;

            let htf_candles = endpoints::get_candles_paginated(
                &self.oanda_client,
                &self.config.instrument,
                htf_gran,
                &from_time,
                &to_time,
            ).await
            .map_err(|e| crate::error::Error::Strategy(format!("Failed to fetch {} candles: {}", tf, e)))?;

            info!(
                "[Watcher {}] Fetched {} warmup candles for HTF timeframe {}",
                self.config_id, htf_candles.len(), tf
            );
            mtf_store.add_timeframe(tf.clone(), htf_candles);
        }

        self.executor.set_mtf_candle_store(mtf_store);
        self.last_htf_refresh = Some(Instant::now());
        Ok(())
    }

    /// Refresh HTF candles during live trading.
    /// Fetches the latest few candles for each HTF timeframe and appends any new ones
    /// to the existing store. Rate-limited to avoid excessive API calls.
    async fn refresh_htf_candles(&mut self) -> Result<()> {
        use std::str::FromStr;

        // Rate-limit: only refresh every 5 minutes
        const HTF_REFRESH_INTERVAL: Duration = Duration::from_secs(300);
        if let Some(last) = self.last_htf_refresh {
            if last.elapsed() < HTF_REFRESH_INTERVAL {
                return Ok(());
            }
        }

        for tf in self.htf_timeframes.clone() {
            let htf_gran = Granularity::from_str(&tf)
                .map_err(|e| crate::error::Error::Strategy(format!("Invalid HTF granularity '{}': {}", tf, e)))?;

            // Fetch the latest 3 candles to catch any newly completed ones
            let htf_candles = endpoints::get_candles(
                &self.oanda_client,
                &self.config.instrument,
                htf_gran,
                Some(3),
                None,
                None,
            ).await
            .map_err(|e| crate::error::Error::Strategy(format!("Failed to refresh {} candles: {}", tf, e)))?;

            // Append only complete candles that are newer than what's in the store.
            // These will be picked up by advance_htf_engines on the next on_candle_live call.
            for candle in htf_candles {
                if candle.complete {
                    self.executor.append_htf_candle(&tf, candle);
                }
            }
        }

        self.last_htf_refresh = Some(Instant::now());
        Ok(())
    }

    /// Process a single tick (poll for new candle and evaluate)
    async fn process_tick(&mut self, sink: &dyn EventSink) -> Result<()> {
        // Check for new candle
        let candle = match self.candle_source.get_latest_candle().await? {
            Some(c) => {
                info!(
                    "[Watcher {}] New candle: time={}, O={}, H={}, L={}, C={}",
                    self.config_id,
                    c.time,
                    c.mid.open,
                    c.mid.high,
                    c.mid.low,
                    c.mid.close
                );
                c
            }
            None => return Ok(()), // No new candle
        };

        // Refresh HTF candles periodically so newly completed HTF candles
        // are available for the MtfCandleStore advance mechanism
        if !self.htf_timeframes.is_empty() {
            if let Err(e) = self.refresh_htf_candles().await {
                warn!("[Watcher {}] Failed to refresh HTF candles: {}", self.config_id, e);
                // Non-fatal: continue with existing HTF data
            }
        }

        // Increment candle count on all pending signals and check for expired ones
        self.update_pending_signals(sink);

        // Check current position state from OANDA (the source of truth)
        let position_direction = self.check_open_position().await?;

        // Process candle through strategy executor using live mode
        // This uses the actual broker position state instead of internal tracking
        let signal = self.executor.on_candle_live(&candle, position_direction);

        // A scripted strategy that just hit its consecutive-error abort threshold
        // reports it here — surface it as a distinct health event, not a silent Hold.
        if let Some(reason) = self.executor.take_health_event() {
            self.emit_error(sink, "script_aborted", &reason);
        }

        // Capture indicator snapshot for auto-notes
        let indicator_snapshot = self.executor.get_indicator_snapshot();

        // Check if new signal invalidates existing pending signals
        self.check_signal_conflicts(&signal, sink);

        // Determine signal result for debug event
        let signal_result = match &signal {
            crate::backtest::rules_engine::RulesSignal::Hold => "Hold".to_string(),
            crate::backtest::rules_engine::RulesSignal::Entry { direction, .. } => {
                format!("Entry {:?}", direction)
            }
            crate::backtest::rules_engine::RulesSignal::Exit { .. } => "Exit".to_string(),
            crate::backtest::rules_engine::RulesSignal::PartialExit { .. } => "PartialExit".to_string(),
        };

        // Emit debug tick event so UI can see candles being processed
        let tick_event = WatcherTickEvent {
            config_id: self.config_id.clone(),
            instrument: self.config.instrument.clone(),
            timeframe: self.config.timeframe.to_string(),
            candle_time: candle.time.to_rfc3339(),
            close_price: candle.mid.close.to_string(),
            signal_result: signal_result.clone(),
            backfill: false,
        };
        sink.watcher_tick(&tick_event);

        // Filter and emit signal based on position state
        if let Some(strategy_signal) = self.filter_signal(signal, position_direction, &candle, Some(indicator_snapshot)).await {
            info!(
                "[Watcher {}] Signal generated: {:?} {:?}, pending_count={}",
                self.config_id,
                strategy_signal.match_type,
                strategy_signal.direction,
                self.pending_signals.len()
            );

            // Track the signal for TTL and conflict detection
            // Note: We no longer block "duplicate" signals - each candle that matches
            // should emit its own signal. Users can dismiss or execute as they choose.
            self.pending_signals.push(PendingSignal {
                signal: strategy_signal.clone(),
                candles_since: 0,
            });
            self.emit_signal(sink, strategy_signal);
        }

        Ok(())
    }

    /// Update pending signals: increment candle count and expire old ones
    fn update_pending_signals(&mut self, sink: &dyn EventSink) {
        let ttl = self.signal_ttl_candles;

        // Collect signals to expire
        let mut expired_ids = Vec::new();

        for pending in &mut self.pending_signals {
            pending.candles_since += 1;

            if pending.candles_since >= ttl {
                expired_ids.push(pending.signal.id.clone());
            }
        }

        // Expire and emit updates for old signals
        for signal_id in &expired_ids {
            self.emit_signal_status_update(
                sink,
                signal_id.clone(),
                SignalStatus::Expired,
                format!("Signal expired after {} candles without execution", ttl),
            );
        }

        // Remove expired signals
        self.pending_signals
            .retain(|p| !expired_ids.contains(&p.signal.id));
    }

    /// Check if a new signal conflicts with pending signals and invalidate them
    fn check_signal_conflicts(&mut self, new_signal: &RulesSignal, sink: &dyn EventSink) {
        let mut signals_to_expire = Vec::new();

        match new_signal {
            // Exit signal invalidates pending entry signals
            RulesSignal::Exit { .. } | RulesSignal::PartialExit { .. } => {
                for pending in &self.pending_signals {
                    if pending.signal.match_type == SignalType::Entry {
                        signals_to_expire.push((
                            pending.signal.id.clone(),
                            "Exit signal generated - entry no longer valid".to_string(),
                        ));
                    }
                }
            }

            // New entry signal in opposite direction invalidates pending entry signals
            RulesSignal::Entry { direction, .. } => {
                for pending in &self.pending_signals {
                    if pending.signal.match_type == SignalType::Entry {
                        if let Some(pending_dir) = &pending.signal.direction {
                            if pending_dir != direction {
                                signals_to_expire.push((
                                    pending.signal.id.clone(),
                                    format!(
                                        "Opposite direction signal generated ({:?} vs {:?})",
                                        direction, pending_dir
                                    ),
                                ));
                            }
                        }
                    }
                }
            }

            RulesSignal::Hold => {}
        }

        // Expire conflicting signals
        for (signal_id, reason) in &signals_to_expire {
            self.emit_signal_status_update(
                sink,
                signal_id.clone(),
                SignalStatus::Expired,
                reason.clone(),
            );
        }

        // Remove expired signals
        let ids_to_remove: Vec<_> = signals_to_expire.iter().map(|(id, _)| id.clone()).collect();
        self.pending_signals
            .retain(|p| !ids_to_remove.contains(&p.signal.id));
    }

    /// Emit a signal status update event to the frontend
    fn emit_signal_status_update(
        &self,
        sink: &dyn EventSink,
        signal_id: String,
        new_status: SignalStatus,
        reason: String,
    ) {
        let event = SignalStatusUpdateEvent {
            match_id: signal_id,
            config_id: self.config_id.clone(),
            new_status,
            reason,
        };
        sink.match_status_update(&event);
    }

    /// Check if there's an open position for this instrument and return its direction
    async fn check_open_position(&self) -> Result<Option<PositionDirection>> {
        let positions = get_open_positions(&self.oanda_client).await?;

        for p in positions {
            if p.instrument == self.config.instrument && !p.is_flat() {
                // Determine direction based on units sign (positive = long, negative = short)
                if p.units > rust_decimal::Decimal::ZERO {
                    return Ok(Some(PositionDirection::Long));
                } else {
                    return Ok(Some(PositionDirection::Short));
                }
            }
        }
        Ok(None)
    }

    /// Filter signal based on position state and execution mode
    async fn filter_signal(
        &self,
        signal: RulesSignal,
        position_direction: Option<PositionDirection>,
        candle: &crate::models::Candle,
        indicator_snapshot: Option<IndicatorSnapshot>,
    ) -> Option<StrategySignal> {
        let has_position = position_direction.is_some();

        match signal {
            RulesSignal::Hold => None,

            RulesSignal::Entry { direction, stop_loss, take_profit, .. } => {
                // Always generate entry signals - user can decide whether to scale in
                // or add to existing positions. We pass has_position so UI can inform user.
                let entry_price = candle.mid.close;
                // A script that emits no stop_loss/take_profit manages its own
                // exits — pass the bracket through as None so the order carries
                // no on-fill SL/TP. Defaulting to entry_price yields SL==TP==entry,
                // which OANDA rejects with TAKE_PROFIT_ON_FILL_LOSS.
                let sl = stop_loss;
                let tp = take_profit;

                // Calculate position size based on risk settings and stop distance.
                // Sizing needs a concrete stop; fall back to entry_price for that math only.
                let position_size = self.calculate_position_size_for_signal(entry_price, sl.unwrap_or(entry_price), direction).await;

                let reason = format!(
                    "{} entry signal at {}",
                    match direction {
                        PositionDirection::Long => "Long",
                        PositionDirection::Short => "Short",
                    },
                    entry_price
                );

                Some(StrategySignal::entry(
                    self.config.user_id.clone(),
                    self.config_id.clone(),
                    self.config.instrument.clone(),
                    direction,
                    entry_price,
                    sl,
                    tp,
                    position_size,
                    reason,
                    indicator_snapshot,
                    has_position, // Pass position state to UI for informational display
                ))
            }

            RulesSignal::Exit { reason, .. } => {
                // Only generate exit signal if in position
                let pos_dir = position_direction?;

                Some(StrategySignal::exit(
                    self.config.user_id.clone(),
                    self.config_id.clone(),
                    self.config.instrument.clone(),
                    pos_dir,
                    reason,
                    indicator_snapshot,
                ))
            }

            RulesSignal::PartialExit { reason, close_percent, .. } => {
                // Only generate partial exit signal if in position
                let pos_dir = position_direction?;

                Some(StrategySignal::partial_exit(
                    self.config.user_id.clone(),
                    self.config_id.clone(),
                    self.config.instrument.clone(),
                    pos_dir,
                    close_percent,
                    reason,
                    indicator_snapshot,
                ))
            }
        }
    }

    /// Calculate position size for an entry signal by fetching account balance
    /// and using the strategy's risk settings.
    async fn calculate_position_size_for_signal(
        &self,
        entry_price: rust_decimal::Decimal,
        stop_loss: rust_decimal::Decimal,
        direction: PositionDirection,
    ) -> Option<rust_decimal::Decimal> {
        // Fetch account balance from OANDA
        let account = match get_account(&self.oanda_client).await {
            Ok(acc) => acc,
            Err(e) => {
                warn!("[Watcher {}] Failed to fetch account for position sizing: {}", self.config_id, e);
                return None;
            }
        };

        // Parse balance
        let balance: rust_decimal::Decimal = match account.balance.parse() {
            Ok(b) => b,
            Err(e) => {
                warn!("[Watcher {}] Failed to parse account balance '{}': {}", self.config_id, account.balance, e);
                return None;
            }
        };

        // Calculate position size using strategy executor (uses direction-specific risk settings)
        // For scripted strategies this returns None — sizing comes from the script itself.
        let position_size = self.executor.calculate_position_size(balance, entry_price, stop_loss, direction);

        if let Some(size) = position_size {
            info!(
                "[Watcher {}] Calculated position size: {} units (balance: {}, entry: {}, stop: {})",
                self.config_id, size, balance, entry_price, stop_loss
            );
        }

        position_size
    }

    /// Emit a signal event to the sink
    fn emit_signal(&self, sink: &dyn EventSink, signal: StrategySignal) {
        // Emit event to the sink
        let event = StrategySignalEvent {
            pattern_match: signal,
            strategy_name: self.config.strategy_name.clone(),
            timeframe: self.config.timeframe.to_string(),
        };
        sink.pattern_matched(&event);
    }

    /// Emit a status event to the sink
    fn emit_status(&self, sink: &dyn EventSink, status: WatcherStatus, message: Option<String>) {
        let event = StrategyStatusEvent {
            config_id: self.config_id.clone(),
            status,
            message,
        };
        sink.strategy_status(&event);
    }

    /// Emit an error event to the sink
    fn emit_error(&self, sink: &dyn EventSink, error_type: &str, message: &str) {
        let event = StrategyErrorEvent {
            config_id: self.config_id.clone(),
            error_type: error_type.to_string(),
            message: message.to_string(),
        };
        sink.strategy_error(&event);
    }
}

/// Status information about an active watcher
#[derive(Debug, Clone, serde::Serialize)]
pub struct WatcherInfo {
    pub config_id: String,
    pub instrument: String,
    pub timeframe: String,
    pub is_running: bool,
}

impl From<&StrategyWatcher> for WatcherInfo {
    fn from(watcher: &StrategyWatcher) -> Self {
        Self {
            config_id: watcher.config_id.clone(),
            instrument: watcher.instrument().to_string(),
            timeframe: watcher.timeframe().to_string(),
            is_running: watcher.is_running(),
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_mode_from_str() {
        assert_eq!(
            "signal_only".parse::<ExecutionMode>().unwrap(),
            ExecutionMode::SignalOnly
        );
        assert_eq!(
            "confirm_execute".parse::<ExecutionMode>().unwrap(),
            ExecutionMode::ConfirmExecute
        );
        assert_eq!(
            "auto_execute".parse::<ExecutionMode>().unwrap(),
            ExecutionMode::AutoExecute
        );
        assert!("invalid".parse::<ExecutionMode>().is_err());
    }

    #[test]
    fn test_watcher_config_default() {
        let config = WatcherConfig::default();
        assert_eq!(config.instrument, "EUR_USD");
        assert_eq!(config.timeframe, Granularity::H1);
        assert_eq!(config.warmup_candles, 100);
    }
}
