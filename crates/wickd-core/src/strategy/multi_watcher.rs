//! Multi-Instrument Strategy Watcher
//!
//! A consolidated watcher that handles multiple instruments for a single
//! (strategy + timeframe) combination. This reduces thread count from
//! N instruments to 1 watcher per strategy/timeframe pair.
//!
//! Key features:
//! - Dynamic add/remove instruments via command channel
//! - Separate RulesEngine per instrument (pattern matches emit individually)
//! - Shared poll interval based on timeframe
//! - Single OS thread per watcher

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, oneshot};
use tokio::time::sleep;
use tracing::{info, warn};

use crate::backtest::rules_engine::{PositionDirection, RulesEngine, RulesSignal, SRZone, StrategyDefinition};
use crate::backtest::scripted_strategy::ScriptedStrategy;
use crate::backtest::surprise::SurpriseCalendar;
use crate::error::Result;
use crate::models::Candle;
use crate::oanda::client::OandaClient;
use crate::oanda::endpoints::{get_account, get_open_positions, Granularity};

use super::candle_boundary::CandleBoundaryService;
use super::candle_source::{CandleSource, OandaPollingSource, StreamingCandleSource};
use super::pattern_match::{
    IndicatorSnapshot, StrategyErrorEvent, StrategyStatusEvent, WatcherStatus, WatcherTickEvent,
    PatternMatch as StrategySignal, PatternMatchEvent as StrategySignalEvent,
    MatchType as SignalType, MatchStatus as SignalStatus,
    MatchStatusUpdateEvent as SignalStatusUpdateEvent,
};
use super::watch_state::WatchStateStore;
use super::watcher::ExecutionMode;
use crate::event_sink::EventSink;

/// Default number of candles before a signal expires
const DEFAULT_SIGNAL_TTL_CANDLES: u32 = 3;

/// Ceiling on how many missed candles are rule-evaluated at startup
/// (see [`MultiInstrumentWatcher::backfill_instrument`]). Candles beyond
/// this — a multi-day outage — are indicator warmup material again:
/// rule-evaluating a long stale stretch would only produce noise, and the
/// truncation is reported, never silent.
const MAX_BACKFILL_CANDLES: usize = 24;

/// Commands that can be sent to a running watcher
#[derive(Debug)]
pub enum WatcherCommand {
    /// Add a new instrument to watch
    AddInstrument {
        instrument: String,
        sr_zones: Vec<SRZone>,
        signal_filter: String,
        response: oneshot::Sender<std::result::Result<(), String>>,
    },
    /// Remove an instrument from the watcher
    RemoveInstrument {
        instrument: String,
        response: oneshot::Sender<std::result::Result<(), String>>,
    },
    /// Update S/R zones for an instrument
    UpdateSRZones {
        instrument: String,
        sr_zones: Vec<SRZone>,
        response: oneshot::Sender<std::result::Result<(), String>>,
    },
    /// Update signal filter for an instrument
    UpdateSignalFilter {
        instrument: String,
        signal_filter: String,
        response: oneshot::Sender<std::result::Result<(), String>>,
    },
    /// Stop the watcher
    Stop,
}

/// Tracks a pending signal for invalidation purposes
#[derive(Debug, Clone)]
struct PendingSignal {
    /// The signal that was emitted
    signal: StrategySignal,
    /// Number of candles since this signal was generated
    candles_since: u32,
}

use super::watcher::StrategyExecutor;

/// State for a single instrument within the multi-watcher
struct InstrumentState {
    /// Instrument name (e.g., "EUR_USD")
    instrument: String,
    /// Candle data source for this instrument
    candle_source: Box<dyn CandleSource>,
    /// Strategy executor (rules-based or scripted, separate per instrument)
    executor: StrategyExecutor,
    /// Pending signals that haven't been executed yet
    pending_signals: Vec<PendingSignal>,
    /// Candles that closed while the previous watcher process was down,
    /// stashed by [`Self::warmup`] for the startup backfill replay
    /// (never fed to `warmup_candle` — each candle advances the executor
    /// exactly once).
    pending_backfill: Vec<Candle>,
    /// Whether this instrument has been initialized (warmup complete)
    initialized: bool,
    /// Consecutive error count for this instrument
    consecutive_errors: u32,
    /// Signal filter: 'all' | 'entries' | 'exits' | 'longs' | 'shorts'
    signal_filter: String,
}

impl InstrumentState {
    #[allow(clippy::too_many_arguments)]
    fn new(
        instrument: String,
        candle_source: Box<dyn CandleSource>,
        strategy: &StrategyDefinition,
        sr_zones: Vec<SRZone>,
        signal_filter: String,
        timeframe: &str,
        script_params: &HashMap<String, f64>,
        event_calendar: Option<Vec<chrono::DateTime<chrono::Utc>>>,
        surprise_calendar: Option<SurpriseCalendar>,
    ) -> std::result::Result<Self, String> {
        let executor = match strategy.strategy_type.as_str() {
            "scripted" => {
                let script = strategy.script_content.as_deref()
                    .ok_or_else(|| "Scripted strategy missing script_content".to_string())?;
                // AGT-624 AC2: `--set` parameter overrides flow into every
                // per-instrument script instance, exactly as in backtest.
                let mut scripted =
                    ScriptedStrategy::from_script_with_params(script, &strategy.name, script_params.clone())?;
                scripted.set_pip_value_for_instrument(&instrument);
                // Event calendar (ABI v3): feeds hours_since_event()/
                // hours_until_event() in live watch just like backtest does.
                if let Some(events) = event_calendar {
                    scripted.set_event_calendar(events);
                }
                // Surprise feed (ABI v4): feeds surprise_z()/surprise_hours_ago()
                // in live watch just like backtest does; the per-candle refresh
                // hook keeps it current with ~/.wickd/calendar/ CSV drops.
                if let Some(cal) = surprise_calendar {
                    scripted.set_surprise_calendar(cal, &instrument);
                }
                StrategyExecutor::Scripted(scripted)
            }
            _ => {
                // Default: rules-based strategy
                let mut rules_engine = RulesEngine::new(strategy.clone())
                    .map_err(|e| format!("Failed to create rules engine: {}", e))?;

                // Set pip value for the instrument (important for JPY pairs, gold, silver, indices)
                rules_engine.set_pip_value_for_instrument(&instrument);

                // Reclassify indicators whose explicit timeframe matches the chart timeframe
                rules_engine.set_primary_granularity(timeframe);
                rules_engine.set_sr_zones(sr_zones);

                StrategyExecutor::Rules(rules_engine)
            }
        };

        Ok(Self {
            instrument,
            candle_source,
            executor,
            pending_signals: Vec::new(),
            pending_backfill: Vec::new(),
            initialized: false,
            consecutive_errors: 0,
            signal_filter,
        })
    }

    /// Warm up the instrument with historical candles.
    ///
    /// `cutoff` is the last candle this instrument evaluated before the
    /// previous process died (from the [`WatchStateStore`] ledger). Candles
    /// at or before it are warmup material; candles after it were never
    /// rule-evaluated, so they are stashed in `pending_backfill` for the
    /// startup replay instead of being fed here — feeding *and* replaying
    /// would advance the executor's indicators twice on the same candle.
    ///
    /// Returns the newest candle time covered by warmup, so a fresh start
    /// (no ledger entry) can seed the ledger without waiting for a tick.
    async fn warmup(
        &mut self,
        warmup_candles: u32,
        cutoff: Option<DateTime<Utc>>,
    ) -> Result<Option<DateTime<Utc>>> {
        let mut candles = self.candle_source.get_candles(warmup_candles).await?;
        candles.sort_by_key(|c| c.time);

        let mut missed: Vec<Candle> = Vec::new();
        if let Some(cutoff) = cutoff {
            let split = candles.partition_point(|c| c.time <= cutoff);
            missed = candles.split_off(split);
        }

        // A very long outage is warmup material again for all but the newest
        // candles — rule-evaluating a week of stale candles produces noise.
        if missed.len() > MAX_BACKFILL_CANDLES {
            let overflow = missed.len() - MAX_BACKFILL_CANDLES;
            warn!(
                "[{}] outage gap of {} candles exceeds the {}-candle replay cap — the oldest {} are warmup-only (not rule-evaluated)",
                self.instrument,
                missed.len(),
                MAX_BACKFILL_CANDLES,
                overflow
            );
            candles.extend(missed.drain(..overflow));
        }

        for candle in &candles {
            self.executor.warmup_candle(candle);
        }

        // Prime the candle source. A candle that closed between the warmup
        // fetch and now would be swallowed unevaluated — when we have a
        // ledger to compare against, append it to the replay set instead.
        let primed = self.candle_source.get_latest_candle().await?;
        if let (Some(c), Some(cutoff)) = (primed, cutoff) {
            let newest_known = missed
                .last()
                .map(|m| m.time)
                .or_else(|| candles.last().map(|k| k.time));
            if c.complete && c.time > cutoff && newest_known.is_none_or(|t| c.time > t) {
                missed.push(c);
            }
        }

        self.pending_backfill = missed;
        self.initialized = true;
        Ok(candles.last().map(|c| c.time))
    }

    /// Get the latest candle if available
    async fn get_latest_candle(&self) -> Result<Option<Candle>> {
        self.candle_source.get_latest_candle().await
    }

    /// Get recent candles for initial evaluation
    async fn get_recent_candles(&self, count: u32) -> Result<Vec<Candle>> {
        self.candle_source.get_candles(count).await
    }

    /// Process a candle and return the signal
    fn evaluate_candle(&mut self, candle: &Candle, position_direction: Option<PositionDirection>) -> (RulesSignal, IndicatorSnapshot) {
        let signal = self.executor.on_candle_live(candle, position_direction);
        let snapshot = self.executor.get_indicator_snapshot();
        (signal, snapshot)
    }

    /// Update pending signals and return expired signal IDs
    fn update_pending_signals(&mut self, ttl: u32) -> Vec<String> {
        let mut expired_ids = Vec::new();

        for pending in &mut self.pending_signals {
            pending.candles_since += 1;
            if pending.candles_since >= ttl {
                expired_ids.push(pending.signal.id.clone());
            }
        }

        // Remove expired
        self.pending_signals.retain(|p| !expired_ids.contains(&p.signal.id));

        expired_ids
    }

    /// Check for signal conflicts and return IDs to expire with reasons
    fn check_signal_conflicts(&mut self, new_signal: &RulesSignal) -> Vec<(String, String)> {
        let mut signals_to_expire = Vec::new();

        match new_signal {
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

        // Remove conflicting signals
        let ids_to_remove: Vec<_> = signals_to_expire.iter().map(|(id, _)| id.clone()).collect();
        self.pending_signals.retain(|p| !ids_to_remove.contains(&p.signal.id));

        signals_to_expire
    }

    /// Add a pending signal
    fn add_pending_signal(&mut self, signal: StrategySignal) {
        self.pending_signals.push(PendingSignal {
            signal,
            candles_since: 0,
        });
    }
}

/// Multi-instrument strategy watcher
///
/// Watches multiple instruments with a single strategy and timeframe.
/// Each instrument has its own RulesEngine, so pattern matches emit
/// individually per instrument.
pub struct MultiInstrumentWatcher {
    /// Unique ID for this watcher (strategy_id + "_" + timeframe)
    watcher_id: String,
    /// Strategy ID (e.g., from database)
    strategy_id: String,
    /// Strategy name for display
    strategy_name: String,
    /// The strategy definition (cloned for each instrument's RulesEngine)
    strategy: StrategyDefinition,
    /// Candle timeframe
    timeframe: Granularity,
    /// User ID who owns this watcher
    user_id: String,
    /// Per-instrument state (each has its own signal_filter)
    instruments: HashMap<String, InstrumentState>,
    /// OANDA client for position checks and candle fetching
    oanda_client: OandaClient,
    /// Execution mode
    #[allow(dead_code)]
    mode: ExecutionMode,
    /// Command receiver for dynamic changes
    command_rx: mpsc::Receiver<WatcherCommand>,
    /// External stop signal
    stop_signal: Arc<AtomicBool>,
    /// Internal running flag
    running: AtomicBool,
    /// Signal TTL in candles
    signal_ttl_candles: u32,
    /// Number of candles to warm up indicators
    warmup_candles: u32,
    /// Optional candle boundary service for streaming-based detection
    candle_boundary_service: Option<Arc<CandleBoundaryService>>,
    /// Cached position data to avoid N API calls per tick cycle.
    /// When processing multiple instruments in the same cycle, we fetch
    /// positions once and reuse the result for all instruments.
    cached_positions: Option<(Instant, HashMap<String, PositionDirection>)>,
    /// How long to consider cached positions valid (covers a single tick cycle)
    position_cache_ttl: Duration,
    /// Scripted-strategy `@parameters` overrides (AGT-624 AC2). Applied to
    /// every per-instrument `ScriptedStrategy` at construction; ignored for
    /// rules-based strategies. Set via [`Self::set_script_params`] BEFORE
    /// instruments are added.
    script_params: HashMap<String, f64>,
    /// Per-instrument economic-calendar event times for scripted strategies
    /// (feeds `hours_since_event()`/`hours_until_event()`, ABI v3). Set via
    /// [`Self::set_script_event_calendar`] BEFORE the instrument is added.
    script_event_calendars: HashMap<String, Vec<chrono::DateTime<chrono::Utc>>>,
    /// Updatable surprise calendar for scripted strategies (feeds
    /// `surprise_z()`/`surprise_hours_ago()`, ABI v4). One shared load,
    /// cloned into each per-instrument strategy (the instrument's currency
    /// legs become that instance's default filter). Set via
    /// [`Self::set_script_surprise_calendar`] BEFORE instruments are added.
    script_surprise_calendar: Option<SurpriseCalendar>,
    /// Durable per-instrument candle-progress ledger. When set, every
    /// evaluated candle is recorded, and startup replays candles that closed
    /// while the previous process was down (see
    /// [`Self::backfill_instrument`]). `None` preserves the historical
    /// behavior: restarts resume at the current candle, skipping the gap.
    state_store: Option<WatchStateStore>,
}

impl MultiInstrumentWatcher {
    /// Create a new multi-instrument watcher
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        watcher_id: String,
        strategy_id: String,
        strategy_name: String,
        strategy: StrategyDefinition,
        timeframe: Granularity,
        user_id: String,
        oanda_client: OandaClient,
        mode: ExecutionMode,
        command_rx: mpsc::Receiver<WatcherCommand>,
        stop_signal: Arc<AtomicBool>,
    ) -> Self {
        Self {
            watcher_id,
            strategy_id,
            strategy_name,
            strategy,
            timeframe,
            user_id,
            instruments: HashMap::new(),
            oanda_client,
            mode,
            command_rx,
            stop_signal,
            running: AtomicBool::new(false),
            signal_ttl_candles: DEFAULT_SIGNAL_TTL_CANDLES,
            warmup_candles: 100,
            candle_boundary_service: None,
            cached_positions: None,
            position_cache_ttl: Duration::from_secs(5),
            script_params: HashMap::new(),
            script_event_calendars: HashMap::new(),
            script_surprise_calendar: None,
            state_store: None,
        }
    }

    /// Attach the durable candle-progress ledger, enabling restart backfill:
    /// every evaluated candle is recorded, and the next startup replays the
    /// candles that closed while this process was down. Call BEFORE
    /// [`Self::start`]; without it, restarts skip the gap (the historical
    /// behavior).
    pub fn set_state_store(&mut self, store: WatchStateStore) {
        self.state_store = Some(store);
    }

    /// Whether this watcher hosts a scripted (Rhai) strategy. Every instrument
    /// shares `self.strategy`, so the definition's `strategy_type` decides.
    fn is_scripted(&self) -> bool {
        self.strategy.strategy_type == "scripted"
    }

    /// Set the `--set` parameter overrides applied to every per-instrument
    /// scripted-strategy instance (AGT-624 AC2). Callers must invoke this
    /// BEFORE adding instruments; instruments added earlier keep the params
    /// they were built with. No-op for rules-based strategies.
    pub fn set_script_params(&mut self, params: HashMap<String, f64>) {
        self.script_params = params;
    }

    /// Set the economic-calendar event times injected into `instrument`'s
    /// scripted-strategy instance (ABI v3 `hours_since_event()`/
    /// `hours_until_event()`). Callers must invoke this BEFORE adding the
    /// instrument. No-op for rules-based strategies.
    pub fn set_script_event_calendar(
        &mut self,
        instrument: String,
        events: Vec<chrono::DateTime<chrono::Utc>>,
    ) {
        self.script_event_calendars.insert(instrument, events);
    }

    /// Set the surprise calendar injected into every per-instrument
    /// scripted-strategy instance (ABI v4 `surprise_z()` family). Callers
    /// must invoke this BEFORE adding instruments. No-op for rules-based
    /// strategies.
    pub fn set_script_surprise_calendar(&mut self, calendar: SurpriseCalendar) {
        self.script_surprise_calendar = Some(calendar);
    }

    /// Set the candle boundary service for streaming-based candle detection.
    ///
    /// When set, the watcher will use `StreamingCandleSource` instead of
    /// `OandaPollingSource`, reducing candle detection latency from minutes
    /// to sub-second.
    pub fn set_candle_boundary_service(&mut self, service: Arc<CandleBoundaryService>) {
        info!(
            "[MultiWatcher:{}] Streaming mode enabled with boundary service",
            self.watcher_id
        );
        self.candle_boundary_service = Some(service);
    }

    /// Check if streaming mode is enabled
    pub fn is_streaming(&self) -> bool {
        self.candle_boundary_service.is_some()
    }

    /// Add an instrument to watch
    ///
    /// If the candle boundary service is configured, uses `StreamingCandleSource`
    /// for sub-second candle detection. Otherwise, falls back to `OandaPollingSource`.
    pub async fn add_instrument(&mut self, instrument: String, sr_zones: Vec<SRZone>, signal_filter: String) -> std::result::Result<(), String> {
        if self.instruments.contains_key(&instrument) {
            return Err(format!("Instrument {} already being watched", instrument));
        }

        // Use streaming source if boundary service is available, otherwise fall back to polling
        let candle_source: Box<dyn CandleSource> = if let Some(ref service) = self.candle_boundary_service {
            info!(
                "[MultiWatcher:{}] Using streaming source for {}",
                self.watcher_id, instrument
            );
            Box::new(StreamingCandleSource::new(
                self.oanda_client.clone(),
                instrument.clone(),
                self.timeframe,
                service,
            ).await)
        } else {
            Box::new(OandaPollingSource::new(
                self.oanda_client.clone(),
                instrument.clone(),
                self.timeframe,
            ))
        };

        let state = InstrumentState::new(
            instrument.clone(),
            candle_source,
            &self.strategy,
            sr_zones,
            signal_filter,
            &self.timeframe.to_string(),
            &self.script_params,
            self.script_event_calendars.get(&instrument).cloned(),
            self.script_surprise_calendar.clone(),
        )?;

        self.instruments.insert(instrument, state);
        Ok(())
    }

    /// Add an instrument backed by a caller-provided [`CandleSource`].
    ///
    /// Unlike [`add_instrument`](Self::add_instrument) — which internally picks
    /// a polling or (boundary-service) streaming source — this lets the caller
    /// supply the source directly. `wickd watch` uses it to wire a hub-socket
    /// tick source (AGT-618 AC2) so N watchers share the one upstream streaming
    /// subscription instead of each opening its own.
    pub async fn add_instrument_with_source(
        &mut self,
        instrument: String,
        candle_source: Box<dyn CandleSource>,
        sr_zones: Vec<SRZone>,
        signal_filter: String,
    ) -> std::result::Result<(), String> {
        if self.instruments.contains_key(&instrument) {
            return Err(format!("Instrument {} already being watched", instrument));
        }

        let state = InstrumentState::new(
            instrument.clone(),
            candle_source,
            &self.strategy,
            sr_zones,
            signal_filter,
            &self.timeframe.to_string(),
            &self.script_params,
            self.script_event_calendars.get(&instrument).cloned(),
            self.script_surprise_calendar.clone(),
        )?;

        self.instruments.insert(instrument, state);
        Ok(())
    }

    /// Update signal filter for an instrument
    pub fn update_signal_filter(&mut self, instrument: &str, signal_filter: String) -> std::result::Result<(), String> {
        if let Some(state) = self.instruments.get_mut(instrument) {
            state.signal_filter = signal_filter;
            Ok(())
        } else {
            Err(format!("Instrument {} not found", instrument))
        }
    }

    /// Remove an instrument from the watcher
    pub fn remove_instrument(&mut self, instrument: &str) -> std::result::Result<(), String> {
        if self.instruments.remove(instrument).is_some() {
            Ok(())
        } else {
            Err(format!("Instrument {} not found", instrument))
        }
    }

    /// Get the list of instruments being watched
    pub fn instruments(&self) -> Vec<String> {
        self.instruments.keys().cloned().collect()
    }

    /// Check if we should stop
    fn should_stop(&self) -> bool {
        !self.running.load(Ordering::SeqCst) || self.stop_signal.load(Ordering::SeqCst)
    }

    /// Get the poll interval based on timeframe and streaming mode
    ///
    /// When streaming is enabled, uses a short interval since candle detection
    /// is event-driven (we're just checking the channel). When polling, uses
    /// longer intervals based on timeframe.
    fn poll_interval(&self) -> Duration {
        // With streaming enabled, poll frequently since we're just checking the channel
        if self.is_streaming() {
            return Duration::from_secs(1);
        }

        // Polling mode: interval based on timeframe
        match self.timeframe {
            Granularity::S5 | Granularity::S10 | Granularity::S15 | Granularity::S30 => {
                Duration::from_secs(5)
            }
            Granularity::M1 => Duration::from_secs(10),
            Granularity::M2 | Granularity::M4 | Granularity::M5 => Duration::from_secs(30),
            Granularity::M10 | Granularity::M15 => Duration::from_secs(60),
            Granularity::M30 => Duration::from_secs(120),
            Granularity::H1 => Duration::from_secs(300),
            Granularity::H2 | Granularity::H3 | Granularity::H4 => Duration::from_secs(600),
            Granularity::H6 | Granularity::H8 | Granularity::H12 => Duration::from_secs(900),
            Granularity::D => Duration::from_secs(1800),
            Granularity::W | Granularity::M => Duration::from_secs(3600),
        }
    }

    /// Fetch all open positions and cache them for the current tick cycle.
    ///
    /// This avoids making N API calls when processing N instruments in the same cycle.
    /// With 28 instruments on M1, this reduces API calls from 28 per minute to 1.
    async fn refresh_position_cache(&mut self) -> Result<()> {
        // Check if cache is still valid
        if let Some((cached_at, _)) = &self.cached_positions {
            if cached_at.elapsed() < self.position_cache_ttl {
                return Ok(());
            }
        }

        let positions = get_open_positions(&self.oanda_client).await?;

        let mut position_map = HashMap::new();
        for p in positions {
            if !p.is_flat() {
                let direction = if p.units > rust_decimal::Decimal::ZERO {
                    PositionDirection::Long
                } else {
                    PositionDirection::Short
                };
                position_map.insert(p.instrument.clone(), direction);
            }
        }

        if !position_map.is_empty() {
            info!(
                "[MultiWatcher:{}] Position cache refreshed: {} positions",
                self.watcher_id, position_map.len()
            );
        }

        self.cached_positions = Some((Instant::now(), position_map));
        Ok(())
    }

    /// Check if there's an open position for an instrument using the cached data.
    /// Returns None if no position, Some(direction) if position exists.
    fn get_cached_position(&self, instrument: &str) -> Option<PositionDirection> {
        self.cached_positions
            .as_ref()
            .and_then(|(_, map)| map.get(instrument).cloned())
    }

    /// Invalidate the position cache (e.g., after a new tick cycle starts)
    fn invalidate_position_cache(&mut self) {
        self.cached_positions = None;
    }

    /// Check if an error is transient
    fn is_transient_error(error_msg: &str) -> bool {
        let transient_patterns = [
            "502 Bad Gateway", "503 Service", "504 Gateway",
            "connection refused", "Connection refused",
            "connection reset", "Connection reset",
            "timed out", "timeout", "Timeout",
            "temporarily unavailable", "Too Many Requests", "rate limit",
            "ECONNRESET", "ETIMEDOUT", "ENOTFOUND",
            "network", "Network", "socket", "Socket",
        ];
        transient_patterns.iter().any(|pattern| error_msg.contains(pattern))
    }

    /// Start the watcher's main loop
    pub async fn start(&mut self, sink: Arc<dyn EventSink>) -> Result<()> {
        let sink = sink.as_ref();
        // Use compare_exchange to atomically check and set running state
        if self.running.compare_exchange(
            false,
            true,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ).is_err() {
            return Ok(());
        }

        info!(
            "[MultiWatcher {}] Starting with {} instruments",
            self.watcher_id,
            self.instruments.len()
        );

        // Emit started status
        self.emit_status(sink, WatcherStatus::Running, None);

        // Warm up all instruments
        self.warmup_all(sink).await;

        // Perform initial evaluation on all instruments
        self.evaluate_all_initial(sink).await;

        let poll_interval = self.poll_interval();

        // Main loop
        while !self.should_stop() {
            // Process commands
            self.process_commands(sink).await;

            // If no instruments, wait for commands
            if self.instruments.is_empty() {
                sleep(Duration::from_millis(100)).await;
                continue;
            }

            // Invalidate position cache at the start of each cycle.
            // It will be lazily refreshed on the first instrument that needs it.
            self.invalidate_position_cache();

            // Process all instruments
            let instrument_names: Vec<String> = self.instruments.keys().cloned().collect();
            for instrument in instrument_names {
                if self.should_stop() {
                    break;
                }

                match self.process_instrument_tick(sink, &instrument).await {
                    Ok(()) => {
                        // Reset errors on success
                        if let Some(state) = self.instruments.get_mut(&instrument) {
                            if state.consecutive_errors > 0 {
                                info!(
                                    "[MultiWatcher {}] {} recovered after {} errors",
                                    self.watcher_id, instrument, state.consecutive_errors
                                );
                                state.consecutive_errors = 0;
                            }
                        }
                    }
                    Err(e) => {
                        self.handle_instrument_error(sink, &instrument, &e.to_string());
                    }
                }
            }

            sleep(poll_interval).await;
        }

        // Clean up
        self.running.store(false, Ordering::SeqCst);
        self.emit_status(sink, WatcherStatus::Stopped, None);

        info!("[MultiWatcher {}] Stopped", self.watcher_id);
        Ok(())
    }

    /// Handle an instrument error
    fn handle_instrument_error(&mut self, sink: &dyn EventSink, instrument: &str, error_msg: &str) {
        let (consecutive_errors, should_emit) = {
            if let Some(state) = self.instruments.get_mut(instrument) {
                state.consecutive_errors += 1;
                (state.consecutive_errors, state.consecutive_errors >= 3 || !Self::is_transient_error(error_msg))
            } else {
                return;
            }
        };

        if should_emit {
            let error_type = if Self::is_transient_error(error_msg) {
                "transient_error"
            } else {
                "tick_error"
            };
            let message = if Self::is_transient_error(error_msg) {
                format!("{} (attempt {})", error_msg, consecutive_errors)
            } else {
                error_msg.to_string()
            };
            self.emit_instrument_error(sink, instrument, error_type, &message);
        }
    }

    /// Process pending commands
    async fn process_commands(&mut self, sink: &dyn EventSink) {
        while let Ok(cmd) = self.command_rx.try_recv() {
            match cmd {
                WatcherCommand::AddInstrument { instrument, sr_zones, signal_filter, response } => {
                    info!("[MultiWatcher {}] Adding instrument {} with filter '{}'", self.watcher_id, instrument, signal_filter);

                    let result = self.add_instrument(instrument.clone(), sr_zones, signal_filter).await;

                    // Send response immediately so frontend doesn't block
                    let _ = response.send(result.clone());

                    // Warm up the new instrument if successful (after response sent)
                    if result.is_ok() {
                        self.warmup_single(sink, &instrument).await;
                    }
                }
                WatcherCommand::RemoveInstrument { instrument, response } => {
                    info!("[MultiWatcher {}] Removing instrument {}", self.watcher_id, instrument);
                    let result = self.remove_instrument(&instrument);
                    let _ = response.send(result);
                }
                WatcherCommand::UpdateSRZones { instrument, sr_zones, response } => {
                    let result = if let Some(state) = self.instruments.get_mut(&instrument) {
                        state.executor.set_sr_zones(sr_zones);
                        Ok(())
                    } else {
                        Err(format!("Instrument {} not found", instrument))
                    };
                    let _ = response.send(result);
                }
                WatcherCommand::UpdateSignalFilter { instrument, signal_filter, response } => {
                    info!("[MultiWatcher {}] Updating signal filter for {} to '{}'", self.watcher_id, instrument, signal_filter);
                    let result = self.update_signal_filter(&instrument, signal_filter);
                    let _ = response.send(result);
                }
                WatcherCommand::Stop => {
                    info!("[MultiWatcher {}] Received stop command", self.watcher_id);
                    self.running.store(false, Ordering::SeqCst);
                    return;
                }
            }
        }
    }

    /// Warm up all instruments
    async fn warmup_all(&mut self, sink: &dyn EventSink) {
        let warmup_candles = self.warmup_candles;

        // Process commands and warm up in a loop until all instruments are initialized
        loop {
            // Process any pending commands (this allows adding instruments during warmup)
            self.process_commands(sink).await;

            // Check if we should stop
            if self.should_stop() {
                break;
            }

            // Find an uninitialized instrument
            let uninitialized: Option<String> = self.instruments
                .iter()
                .find(|(_, state)| !state.initialized)
                .map(|(name, _)| name.clone());

            match uninitialized {
                Some(instrument) => {
                    info!(
                        "[MultiWatcher {}] Warming up {} with {} candles",
                        self.watcher_id, instrument, warmup_candles
                    );

                    // Last evaluated candle from the previous process, if a
                    // ledger is attached — candles after it become the
                    // startup backfill instead of warmup material.
                    let cutoff = self
                        .state_store
                        .as_ref()
                        .and_then(|s| s.last_evaluated(&instrument));

                    // Warmup and capture any error
                    let mut covered: Option<DateTime<Utc>> = None;
                    let warmup_error: Option<String> = if let Some(state) = self.instruments.get_mut(&instrument) {
                        match state.warmup(warmup_candles, cutoff).await {
                            Ok(t) => {
                                covered = t;
                                None
                            }
                            Err(e) => {
                                warn!(
                                    "[MultiWatcher {}] Failed to warm up {}: {}",
                                    self.watcher_id, instrument, e
                                );
                                // Mark as initialized anyway to avoid infinite loop
                                state.initialized = true;
                                Some(e.to_string())
                            }
                        }
                    } else {
                        None
                    };

                    // Seed the ledger with the newest warmup-covered candle so
                    // a crash before the first tick still leaves a resume
                    // point (no-op when the ledger is already further along).
                    if let (Some(store), Some(t)) = (self.state_store.as_mut(), covered) {
                        store.record(&instrument, t);
                    }

                    // Emit error after releasing the mutable borrow
                    if let Some(error_msg) = warmup_error {
                        self.emit_instrument_error(sink, &instrument, "warmup_failed", &error_msg);
                    }
                }
                None => {
                    // All instruments are initialized
                    break;
                }
            }
        }
    }

    /// Warm up a single instrument
    async fn warmup_single(&mut self, sink: &dyn EventSink, instrument: &str) {
        let warmup_candles = self.warmup_candles;

        // Last evaluated candle from the previous process, if a ledger is
        // attached (see warmup_all).
        let cutoff = self
            .state_store
            .as_ref()
            .and_then(|s| s.last_evaluated(instrument));

        // Warmup and capture any error
        let mut covered: Option<DateTime<Utc>> = None;
        let warmup_error: Option<String> = if let Some(state) = self.instruments.get_mut(instrument) {
            info!(
                "[MultiWatcher {}] Warming up {} with {} candles",
                self.watcher_id, instrument, warmup_candles
            );

            match state.warmup(warmup_candles, cutoff).await {
                Ok(t) => {
                    covered = t;
                    None
                }
                Err(e) => {
                    warn!(
                        "[MultiWatcher {}] Failed to warm up {}: {}",
                        self.watcher_id, instrument, e
                    );
                    Some(e.to_string())
                }
            }
        } else {
            None
        };

        // Seed the ledger with the newest warmup-covered candle (see warmup_all).
        if let (Some(store), Some(t)) = (self.state_store.as_mut(), covered) {
            store.record(instrument, t);
        }

        // Emit error after releasing the mutable borrow
        if let Some(error_msg) = warmup_error {
            self.emit_instrument_error(sink, instrument, "warmup_failed", &error_msg);
        }

        // An instrument added mid-run replays any gap right away — its
        // initial-evaluation slot has already passed.
        if let Err(e) = self.backfill_instrument(sink, instrument).await {
            warn!(
                "[MultiWatcher {}] Backfill failed for {}: {}",
                self.watcher_id, instrument, e
            );
        }
    }

    /// Perform initial evaluation on all instruments
    async fn evaluate_all_initial(&mut self, sink: &dyn EventSink) {
        // Process commands between each instrument evaluation to stay responsive
        loop {
            // Process any pending commands
            self.process_commands(sink).await;

            if self.should_stop() {
                break;
            }

            // Find an initialized instrument that hasn't been evaluated yet
            // We track this by checking if it has any pending signals (none = not evaluated)
            let instrument_names: Vec<String> = self.instruments.keys().cloned().collect();
            let mut evaluated_any = false;

            for instrument in instrument_names {
                // Process commands before each evaluation
                self.process_commands(sink).await;

                if self.should_stop() {
                    return;
                }

                // A restart gap stashed by warmup takes the initial-evaluation
                // slot: the replay's newest candle IS the current market
                // state, so running both would evaluate it twice.
                let has_backfill = self
                    .instruments
                    .get(&instrument)
                    .is_some_and(|s| !s.pending_backfill.is_empty());

                let result = if has_backfill {
                    self.backfill_instrument(sink, &instrument).await
                } else {
                    self.evaluate_instrument_initial(sink, &instrument).await
                };

                if let Err(e) = result {
                    warn!(
                        "[MultiWatcher {}] Initial evaluation failed for {}: {}",
                        self.watcher_id, instrument, e
                    );
                }
                evaluated_any = true;
            }

            // If we evaluated all instruments (or there were none), we're done
            if evaluated_any || self.instruments.is_empty() {
                break;
            }
        }
    }

    /// Evaluate initial state for a single instrument
    async fn evaluate_instrument_initial(&mut self, sink: &dyn EventSink, instrument: &str) -> Result<()> {
        // Get candles first (releases borrow)
        let (candles, initialized) = {
            let state = self.instruments.get(instrument)
                .ok_or_else(|| crate::error::Error::Strategy(format!("Instrument {} not found", instrument)))?;

            if !state.initialized {
                return Ok(());
            }

            (state.get_recent_candles(2).await?, true)
        };

        if !initialized {
            return Ok(());
        }

        let candle = candles
            .iter()
            .filter(|c| c.complete)
            .max_by_key(|c| c.time)
            .cloned()
            .ok_or_else(|| crate::error::Error::Strategy("No complete candle available".to_string()))?;

        info!(
            "[MultiWatcher {}] Initial evaluation for {} on candle: time={}, C={}",
            self.watcher_id, instrument, candle.time, candle.mid.close
        );

        // Check position direction using cached data (1 API call per cycle, not per instrument)
        self.refresh_position_cache().await?;
        let position_direction = self.get_cached_position(instrument);

        // Evaluate rules (uses &mut instrument state). A scripted strategy that
        // just hit its consecutive-error abort threshold reports it via
        // take_health_event() in the same borrow — surface it as a distinct
        // health event, not a silent Hold.
        let (signal, indicator_snapshot, health_event) = {
            let state = self.instruments.get_mut(instrument).unwrap();
            let (signal, snapshot) = state.evaluate_candle(&candle, position_direction);
            let health_event = state.executor.take_health_event();
            (signal, snapshot, health_event)
        };

        if let Some(reason) = health_event {
            self.emit_instrument_error(sink, instrument, "script_aborted", &reason);
        }

        // Filter and emit signal
        if let Some(strategy_signal) = self.create_signal(
            &signal,
            position_direction,
            &candle,
            Some(indicator_snapshot),
            instrument,
        ).await {
            info!(
                "[MultiWatcher {}] Initial signal for {}: {:?} {:?}",
                self.watcher_id, instrument, strategy_signal.match_type, strategy_signal.direction
            );

            // Check signal filter BEFORE adding to pending signals or emitting.
            // This prevents filtered-out signals from being tracked (which would
            // block future signals via conflict detection) and avoids the race
            // condition where emit_signal checks a stale filter. (BUG-062)
            let signal_filter = self.instruments
                .get(&strategy_signal.instrument)
                .map(|state| state.signal_filter.clone())
                .unwrap_or_else(|| "all".to_string());

            if !self.should_emit_signal(&strategy_signal, &signal_filter) {
                info!(
                    "[MultiWatcher {}] Initial signal filtered out for {}, signal_filter='{}': {:?} {:?}",
                    self.watcher_id, instrument, signal_filter, strategy_signal.match_type, strategy_signal.direction
                );
            } else {
                // Only add to pending signals if signal passes filter
                if let Some(state) = self.instruments.get_mut(instrument) {
                    state.add_pending_signal(strategy_signal.clone());
                }
                self.emit_signal(sink, strategy_signal);
            }
        }

        // Record progress so a restart resumes from here, not from wherever
        // the last tick happened to be.
        if let Some(store) = self.state_store.as_mut() {
            store.record(instrument, candle.time);
        }

        Ok(())
    }

    /// Replay candles that closed while the watcher process was down.
    ///
    /// Runs once per instrument at startup (taking the initial-evaluation
    /// slot) whenever the [`WatchStateStore`] ledger shows a gap between the
    /// last evaluated candle and now. Every replayed candle goes through the
    /// normal evaluation path and is emitted as a `backfill: true`
    /// watcher-tick, so the NDJSON record has no silent hole.
    ///
    /// Signal policy for replayed candles:
    /// - The NEWEST candle behaves exactly like the regular initial
    ///   evaluation — it is the current market state, and its signal emits
    ///   normally (tradeable under `--auto`).
    /// - An interior Entry is STALE — the market has already moved past its
    ///   close — so it is surfaced as a `missed_signal` strategy-error
    ///   instead of a tradeable signal.
    /// - Interior Exit/PartialExit signals emit normally: acting late on an
    ///   exit is strictly safer than never acting on it.
    async fn backfill_instrument(&mut self, sink: &dyn EventSink, instrument: &str) -> Result<()> {
        let candles = {
            let state = self.instruments.get_mut(instrument).ok_or_else(|| {
                crate::error::Error::Strategy(format!("Instrument {} not found", instrument))
            })?;
            std::mem::take(&mut state.pending_backfill)
        };

        let Some(last) = candles.last() else {
            return Ok(()); // no gap — nothing to replay
        };

        info!(
            "[MultiWatcher {}] {} replaying {} candle(s) that closed while the watcher was down: {} .. {}",
            self.watcher_id,
            instrument,
            candles.len(),
            candles[0].time,
            last.time
        );

        // One position fetch for the whole replay — same source of truth the
        // regular tick path uses.
        self.refresh_position_cache().await?;
        let position_direction = self.get_cached_position(instrument);

        let last_idx = candles.len() - 1;
        for (idx, candle) in candles.iter().enumerate() {
            let is_newest = idx == last_idx;

            let (signal, indicator_snapshot, health_event) = {
                let state = self.instruments.get_mut(instrument).ok_or_else(|| {
                    crate::error::Error::Strategy(format!(
                        "Instrument {} removed mid-backfill",
                        instrument
                    ))
                })?;
                let (signal, snapshot) = state.evaluate_candle(candle, position_direction);
                let health_event = state.executor.take_health_event();
                (signal, snapshot, health_event)
            };

            if let Some(reason) = health_event {
                self.emit_instrument_error(sink, instrument, "script_aborted", &reason);
            }

            let signal_result = match &signal {
                RulesSignal::Hold => "Hold".to_string(),
                RulesSignal::Entry { direction, .. } => format!("Entry {:?}", direction),
                RulesSignal::Exit { .. } => "Exit".to_string(),
                RulesSignal::PartialExit { .. } => "PartialExit".to_string(),
            };

            sink.watcher_tick(&WatcherTickEvent {
                config_id: format!("{}_{}", self.watcher_id, instrument),
                instrument: instrument.to_string(),
                timeframe: self.timeframe.to_string(),
                candle_time: candle.time.to_rfc3339(),
                close_price: candle.mid.close.to_string(),
                signal_result: signal_result.clone(),
                backfill: true,
            });

            let stale_entry = !is_newest && matches!(signal, RulesSignal::Entry { .. });
            if stale_entry {
                warn!(
                    "[MultiWatcher {}] {} missed {} at candle {} (watcher was down) — reported, not emitted",
                    self.watcher_id, instrument, signal_result, candle.time
                );
                self.emit_instrument_error(
                    sink,
                    instrument,
                    "missed_signal",
                    &format!(
                        "{} on backfilled candle {} (closed while the watcher was down) — stale, not emitted as tradeable",
                        signal_result,
                        candle.time.to_rfc3339()
                    ),
                );
            } else if let Some(strategy_signal) = self
                .create_signal(&signal, position_direction, candle, Some(indicator_snapshot), instrument)
                .await
            {
                // Same filter-before-track discipline as the live tick path (BUG-062).
                let signal_filter = self
                    .instruments
                    .get(&strategy_signal.instrument)
                    .map(|state| state.signal_filter.clone())
                    .unwrap_or_else(|| "all".to_string());

                if !self.should_emit_signal(&strategy_signal, &signal_filter) {
                    info!(
                        "[MultiWatcher {}] Backfill signal filtered out for {}, signal_filter='{}': {:?} {:?}",
                        self.watcher_id, instrument, signal_filter, strategy_signal.match_type, strategy_signal.direction
                    );
                } else {
                    if let Some(state) = self.instruments.get_mut(instrument) {
                        state.add_pending_signal(strategy_signal.clone());
                    }
                    self.emit_signal(sink, strategy_signal);
                }
            }

            if let Some(store) = self.state_store.as_mut() {
                store.record(instrument, candle.time);
            }
        }

        Ok(())
    }

    /// Process a single instrument's tick
    async fn process_instrument_tick(&mut self, sink: &dyn EventSink, instrument: &str) -> Result<()> {
        // Get candle first (releases borrow)
        let candle = {
            let state = self.instruments.get(instrument)
                .ok_or_else(|| crate::error::Error::Strategy(format!("Instrument {} not found", instrument)))?;

            if !state.initialized {
                info!("[MultiWatcher {}] {} not initialized yet, skipping", self.watcher_id, instrument);
                return Ok(());
            }

            match state.get_latest_candle().await? {
                Some(c) => c,
                None => return Ok(()), // No new candle
            }
        };

        info!(
            "[MultiWatcher {}] {} new candle: time={}, O={}, H={}, L={}, C={}",
            self.watcher_id, instrument, candle.time,
            candle.mid.open, candle.mid.high, candle.mid.low, candle.mid.close
        );

        // Update pending signals and get expired IDs
        let expired_ids = {
            let state = self.instruments.get_mut(instrument).unwrap();
            state.update_pending_signals(self.signal_ttl_candles)
        };

        // Emit expiry updates
        for signal_id in expired_ids {
            self.emit_match_status_update(
                sink,
                signal_id,
                SignalStatus::Expired,
                format!("Signal expired after {} candles without execution", self.signal_ttl_candles),
            );
        }

        // Check position direction using cached data (1 API call per cycle, not per instrument)
        self.refresh_position_cache().await?;
        let position_direction = self.get_cached_position(instrument);
        info!(
            "[MultiWatcher {}] {} position_direction={:?}",
            self.watcher_id, instrument, position_direction
        );

        // Evaluate rules. A scripted strategy that just hit its consecutive-error
        // abort threshold reports it via take_health_event() in the same borrow —
        // surface it as a distinct health event, not a silent Hold.
        let (signal, indicator_snapshot, health_event) = {
            let state = self.instruments.get_mut(instrument).unwrap();
            let (signal, snapshot) = state.evaluate_candle(&candle, position_direction);
            let health_event = state.executor.take_health_event();
            (signal, snapshot, health_event)
        };
        info!(
            "[MultiWatcher {}] {} signal={:?}",
            self.watcher_id, instrument, signal
        );

        if let Some(reason) = health_event {
            self.emit_instrument_error(sink, instrument, "script_aborted", &reason);
        }

        // Check for signal conflicts
        let conflicts = {
            let state = self.instruments.get_mut(instrument).unwrap();
            state.check_signal_conflicts(&signal)
        };

        // Emit conflict updates
        for (signal_id, reason) in conflicts {
            self.emit_match_status_update(
                sink,
                signal_id,
                SignalStatus::Expired,
                reason,
            );
        }

        // Emit tick event
        let signal_result = match &signal {
            RulesSignal::Hold => "Hold".to_string(),
            RulesSignal::Entry { direction, .. } => format!("Entry {:?}", direction),
            RulesSignal::Exit { .. } => "Exit".to_string(),
            RulesSignal::PartialExit { .. } => "PartialExit".to_string(),
        };

        let tick_event = WatcherTickEvent {
            config_id: format!("{}_{}", self.watcher_id, instrument),
            instrument: instrument.to_string(),
            timeframe: self.timeframe.to_string(),
            candle_time: candle.time.to_rfc3339(),
            close_price: candle.mid.close.to_string(),
            signal_result: signal_result.clone(),
            backfill: false,
        };
        sink.watcher_tick(&tick_event);

        // Record progress so a restart replays exactly the candles after
        // this one (restart backfill), instead of skipping the gap.
        if let Some(store) = self.state_store.as_mut() {
            store.record(instrument, candle.time);
        }

        // Create and emit signal if applicable
        if let Some(strategy_signal) = self.create_signal(
            &signal,
            position_direction,
            &candle,
            Some(indicator_snapshot),
            instrument,
        ).await {
            info!(
                "[MultiWatcher {}] {} signal: {:?} {:?}, pending={}",
                self.watcher_id, instrument, strategy_signal.match_type, strategy_signal.direction,
                self.instruments.get(instrument).map(|s| s.pending_signals.len()).unwrap_or(0)
            );

            // Check signal filter BEFORE adding to pending signals or emitting.
            // This prevents filtered-out signals from being tracked for TTL/conflict
            // detection and avoids race conditions with async filter updates. (BUG-062)
            let signal_filter = self.instruments
                .get(&strategy_signal.instrument)
                .map(|state| state.signal_filter.clone())
                .unwrap_or_else(|| "all".to_string());

            if !self.should_emit_signal(&strategy_signal, &signal_filter) {
                info!(
                    "[MultiWatcher {}] Signal filtered out for {}, signal_filter='{}': {:?} {:?}",
                    self.watcher_id, instrument, signal_filter, strategy_signal.match_type, strategy_signal.direction
                );
            } else {
                // Only add to pending signals if signal passes filter
                // Note: We no longer block "duplicate" signals - each candle that matches
                // should emit its own signal. Users can dismiss or execute as they choose.
                if let Some(state) = self.instruments.get_mut(instrument) {
                    state.add_pending_signal(strategy_signal.clone());
                }
                self.emit_signal(sink, strategy_signal);
            }
        }

        Ok(())
    }

    /// Create a signal based on rules output
    /// Calculate position size for a signal based on the strategy's risk settings.
    /// Fetches the current account balance from OANDA to compute proper risk-based sizing.
    async fn calculate_position_size_for_signal(
        &self,
        entry_price: rust_decimal::Decimal,
        stop_loss: rust_decimal::Decimal,
        direction: PositionDirection,
        instrument: &str,
    ) -> Option<rust_decimal::Decimal> {
        // Scripted strategies size via the script's own stop_loss/take_profit
        // (the executor returns None regardless) — skip the pointless account
        // fetch that would otherwise fire on every scripted entry signal.
        if self.is_scripted() {
            return None;
        }

        let account = match get_account(&self.oanda_client).await {
            Ok(acc) => acc,
            Err(e) => {
                warn!(
                    "[MultiWatcher {}] Failed to get account for position sizing on {}: {}",
                    self.watcher_id, instrument, e
                );
                return None;
            }
        };

        let balance: rust_decimal::Decimal = match account.balance.parse() {
            Ok(b) => b,
            Err(e) => {
                warn!(
                    "[MultiWatcher {}] Failed to parse account balance '{}': {}",
                    self.watcher_id, account.balance, e
                );
                return None;
            }
        };

        if let Some(state) = self.instruments.get(instrument) {
            state
                .executor
                .calculate_position_size(balance, entry_price, stop_loss, direction)
        } else {
            warn!(
                "[MultiWatcher {}] Instrument {} not found in instruments map — skipping position sizing",
                self.watcher_id, instrument
            );
            None
        }
    }

    async fn create_signal(
        &self,
        signal: &RulesSignal,
        position_direction: Option<PositionDirection>,
        candle: &Candle,
        indicator_snapshot: Option<IndicatorSnapshot>,
        instrument: &str,
    ) -> Option<StrategySignal> {
        let has_position = position_direction.is_some();

        match signal {
            RulesSignal::Hold => None,

            RulesSignal::Entry { direction, stop_loss, take_profit, triggered_rule_name, .. } => {
                // Always generate entry signals - user can decide whether to scale in
                // or add to existing positions. We pass has_position so UI can inform user.
                let entry_price = candle.mid.close;
                // A script that emits no stop_loss/take_profit manages its own
                // exits (e.g. opposite-signal reversion) — pass the bracket
                // through as None so the order carries no on-fill SL/TP. Do NOT
                // default to entry_price: SL==TP==entry is a degenerate order
                // OANDA rejects with TAKE_PROFIT_ON_FILL_LOSS.
                let sl = *stop_loss;
                let tp = *take_profit;

                // Calculate position size using the strategy's risk settings and current account balance.
                // Sizing needs a concrete stop distance; fall back to entry_price only for that math
                // (scripted strategies return None here regardless — see calculate_position_size_for_signal).
                let position_size = self
                    .calculate_position_size_for_signal(entry_price, sl.unwrap_or(entry_price), direction.clone(), instrument)
                    .await;

                if position_size.is_none() {
                    warn!(
                        "[MultiWatcher {}] Could not calculate position size for {} on {} — signal will use frontend default",
                        self.watcher_id, match direction { PositionDirection::Long => "long", PositionDirection::Short => "short" }, instrument
                    );
                }

                let reason = format!(
                    "{} entry signal at {}",
                    match direction {
                        PositionDirection::Long => "Long",
                        PositionDirection::Short => "Short",
                    },
                    entry_price
                );

                Some(
                    StrategySignal::entry(
                        self.user_id.clone(),
                        self.strategy_id.clone(),
                        instrument.to_string(),
                        direction.clone(),
                        entry_price,
                        sl,
                        tp,
                        position_size,
                        reason,
                        indicator_snapshot,
                        has_position, // Pass position state to UI for informational display
                    )
                    // AGT-624 AC3: the triggering rule's name (a script's
                    // signal-map `rule_name`, or a rules-based entry rule's
                    // display name) survives into the emitted signal.
                    .with_rule_name(triggered_rule_name.clone()),
                )
            }

            RulesSignal::Exit { reason, .. } => {
                match position_direction {
                    // In position: the exit carries the position's direction.
                    Some(pos_dir) => Some(StrategySignal::exit(
                        self.user_id.clone(),
                        self.strategy_id.clone(),
                        instrument.to_string(),
                        pos_dir,
                        reason.clone(),
                        indicator_snapshot,
                    )),
                    // No broker position. A scripted strategy tracks its
                    // position virtually inside the script, so its close
                    // signals must still surface (AGT-624 AC3) — emit with
                    // direction unknown. Rules-based strategies keep the
                    // existing drop-if-flat behavior unchanged.
                    None if self.is_scripted() => Some(StrategySignal::exit_unpositioned(
                        self.user_id.clone(),
                        self.strategy_id.clone(),
                        instrument.to_string(),
                        reason.clone(),
                        indicator_snapshot,
                    )),
                    None => None,
                }
            }

            RulesSignal::PartialExit { reason, close_percent, .. } => {
                // Only generate partial exit signal if in position
                let pos_dir = position_direction?;

                Some(StrategySignal::partial_exit(
                    self.user_id.clone(),
                    self.strategy_id.clone(),
                    instrument.to_string(),
                    pos_dir,
                    *close_percent,
                    reason.clone(),
                    indicator_snapshot,
                ))
            }
        }
    }

    /// Check if a signal should be emitted based on direction filter (helper for new JSON format)
    fn check_direction_filter(&self, signal: &StrategySignal, filter: &str) -> bool {
        match filter {
            "all" => true,
            "entry" => signal.match_type == SignalType::Entry,
            "exit" => signal.match_type == SignalType::Exit || signal.match_type == SignalType::PartialExit,
            "none" => false,
            _ => true, // Unknown filter, emit
        }
    }

    fn should_emit_signal(&self, signal: &StrategySignal, signal_filter: &str) -> bool {
        // Try to parse as new JSON format: {"long":"all","short":"entry"}
        if signal_filter.starts_with('{') {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(signal_filter) {
                let long_filter = parsed.get("long").and_then(|v| v.as_str()).unwrap_or("all");
                let short_filter = parsed.get("short").and_then(|v| v.as_str()).unwrap_or("all");

                // Determine which filter to apply based on signal direction
                let direction = signal.direction.as_ref();
                return match direction {
                    Some(PositionDirection::Long) => self.check_direction_filter(signal, long_filter),
                    Some(PositionDirection::Short) => self.check_direction_filter(signal, short_filter),
                    None => {
                        // For signals without direction (e.g., some exits), check both filters
                        // Emit if either filter allows it
                        self.check_direction_filter(signal, long_filter) || self.check_direction_filter(signal, short_filter)
                    }
                };
            }
        }

        // Legacy format fallback
        match signal_filter {
            "all" => true,
            "entries" => signal.match_type == SignalType::Entry,
            "exits" => signal.match_type == SignalType::Exit || signal.match_type == SignalType::PartialExit,
            "longs" => signal.match_type == SignalType::Entry && signal.direction == Some(PositionDirection::Long),
            "shorts" => signal.match_type == SignalType::Entry && signal.direction == Some(PositionDirection::Short),
            _ => true, // Unknown filter, emit all
        }
    }

    /// Emit a signal event to the sink
    fn emit_signal(&self, sink: &dyn EventSink, signal: StrategySignal) {
        // Get per-instrument signal filter
        let signal_filter = self.instruments
            .get(&signal.instrument)
            .map(|state| state.signal_filter.as_str())
            .unwrap_or("all");

        // Check signal filter before emitting
        if !self.should_emit_signal(&signal, signal_filter) {
            info!(
                "[MultiWatcher {}] Signal filtered out for {}, signal_filter='{}': {:?} {:?}",
                self.watcher_id, signal.instrument, signal_filter, signal.match_type, signal.direction
            );
            return;
        }

        // Emit event to the sink
        let event = StrategySignalEvent {
            pattern_match: signal.clone(),
            strategy_name: self.strategy_name.clone(),
            timeframe: self.timeframe.to_string(),
        };

        // Store in pending matches so frontend can retrieve if not listening
        // (e.g., if Live Monitor window was closed)
        if let Ok(json) = serde_json::to_value(&event) {
            super::pending_store::add_pending_match(json);
        }

        info!(
            "[MultiWatcher {}] Emitting pattern-matched event: {:?} {:?} for {}",
            self.watcher_id, signal.match_type, signal.direction, signal.instrument
        );
        sink.pattern_matched(&event);
    }

    /// Emit a status event
    fn emit_status(&self, sink: &dyn EventSink, status: WatcherStatus, message: Option<String>) {
        let event = StrategyStatusEvent {
            config_id: self.watcher_id.clone(),
            status,
            message,
        };
        sink.strategy_status(&event);
    }

    /// Emit an error event for a specific instrument
    fn emit_instrument_error(&self, sink: &dyn EventSink, instrument: &str, error_type: &str, message: &str) {
        let event = StrategyErrorEvent {
            config_id: format!("{}_{}", self.watcher_id, instrument),
            error_type: error_type.to_string(),
            message: message.to_string(),
        };
        sink.strategy_error(&event);
    }

    /// Emit a match status update
    fn emit_match_status_update(
        &self,
        sink: &dyn EventSink,
        match_id: String,
        new_status: SignalStatus,
        reason: String,
    ) {
        let event = SignalStatusUpdateEvent {
            match_id,
            config_id: self.watcher_id.clone(),
            new_status,
            reason,
        };
        sink.match_status_update(&event);
    }
}

/// Handle to control a running multi-instrument watcher
#[derive(Clone)]
pub struct MultiWatcherHandle {
    /// Unique watcher ID (strategy_id + "_" + timeframe)
    pub watcher_id: String,
    /// Strategy ID
    pub strategy_id: String,
    /// Strategy name for display
    pub strategy_name: String,
    /// Timeframe
    pub timeframe: String,
    /// Currently watched instruments (dynamically updated)
    pub instruments: Arc<tokio::sync::RwLock<Vec<String>>>,
    /// Command sender for dynamic changes
    pub command_tx: mpsc::Sender<WatcherCommand>,
    /// Stop signal
    pub stop_signal: Arc<AtomicBool>,
}

impl MultiWatcherHandle {
    /// Add an instrument to this watcher
    ///
    /// This is fire-and-forget - we send the command and optimistically update
    /// the local state. The watcher will process the command when it can.
    pub async fn add_instrument(&self, instrument: String, sr_zones: Vec<SRZone>, signal_filter: String) -> std::result::Result<(), String> {
        // Check if already in our list
        {
            let instruments = self.instruments.read().await;
            if instruments.contains(&instrument) {
                return Err(format!("Instrument {} already being watched", instrument));
            }
        }

        // Send command (don't wait for response - watcher may be busy with network calls)
        self.command_tx
            .send(WatcherCommand::AddInstrument {
                instrument: instrument.clone(),
                sr_zones,
                signal_filter,
                response: oneshot::channel().0, // Dummy sender, we won't wait for response
            })
            .await
            .map_err(|_| "Failed to send add command - watcher may have stopped".to_string())?;

        // Optimistically update our local list
        let mut instruments = self.instruments.write().await;
        if !instruments.contains(&instrument) {
            instruments.push(instrument);
        }

        Ok(())
    }

    /// Update signal filter for an instrument
    ///
    /// This is fire-and-forget - changes take effect on next signal check.
    pub async fn update_signal_filter(&self, instrument: String, signal_filter: String) -> std::result::Result<(), String> {
        // Check if in our list
        {
            let instruments = self.instruments.read().await;
            if !instruments.contains(&instrument) {
                return Err(format!("Instrument {} not being watched", instrument));
            }
        }

        // Send command (don't wait for response)
        self.command_tx
            .send(WatcherCommand::UpdateSignalFilter {
                instrument,
                signal_filter,
                response: oneshot::channel().0,
            })
            .await
            .map_err(|_| "Failed to send update filter command - watcher may have stopped".to_string())?;

        Ok(())
    }

    /// Remove an instrument from this watcher
    ///
    /// This is fire-and-forget - we send the command and optimistically update
    /// the local state. The watcher will process the command when it can.
    pub async fn remove_instrument(&self, instrument: &str) -> std::result::Result<(), String> {
        // Check if in our list
        {
            let instruments = self.instruments.read().await;
            if !instruments.contains(&instrument.to_string()) {
                return Err(format!("Instrument {} not being watched", instrument));
            }
        }

        // Send command (don't wait for response)
        self.command_tx
            .send(WatcherCommand::RemoveInstrument {
                instrument: instrument.to_string(),
                response: oneshot::channel().0, // Dummy sender
            })
            .await
            .map_err(|_| "Failed to send remove command - watcher may have stopped".to_string())?;

        // Optimistically update our local list
        let mut instruments = self.instruments.write().await;
        instruments.retain(|i| i != instrument);

        Ok(())
    }

    /// Stop the watcher
    pub async fn stop(&self) -> std::result::Result<(), String> {
        self.stop_signal.store(true, Ordering::SeqCst);
        let _ = self.command_tx.send(WatcherCommand::Stop).await;
        Ok(())
    }

    /// Get current instruments
    pub async fn get_instruments(&self) -> Vec<String> {
        self.instruments.read().await.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BuildMode, Config};
    use crate::shared::{EntryLogic, EntryLogicMode, ParameterizedValue, RiskMethod, RiskSettings};
    use std::future::Future;
    use std::pin::Pin;

    #[test]
    fn test_watcher_id_format() {
        let watcher_id = format!("{}_{}", "strategy_123", "H4");
        assert_eq!(watcher_id, "strategy_123_H4");
    }

    // ---- AGT-624: scripted strategies in the multi-watcher ----

    /// Offline candle source — the scripted-executor tests never touch the
    /// network.
    struct NoopCandleSource;

    impl CandleSource for NoopCandleSource {
        fn get_latest_candle(
            &self,
        ) -> Pin<Box<dyn Future<Output = crate::error::Result<Option<Candle>>> + Send + '_>>
        {
            Box::pin(async { Ok(None) })
        }

        fn get_candles(
            &self,
            _count: u32,
        ) -> Pin<Box<dyn Future<Output = crate::error::Result<Vec<Candle>>> + Send + '_>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn timeframe(&self) -> Granularity {
            Granularity::H1
        }

        fn instrument(&self) -> &str {
            "EUR_USD"
        }

        fn poll_interval(&self) -> Duration {
            Duration::from_secs(1)
        }
    }

    fn dummy_client() -> OandaClient {
        OandaClient::new(&Config {
            api_key: None,
            account_id: None,
            environment: crate::config::OandaEnvironment::Practice,
            anthropic_api_key: None,
            build_mode: BuildMode::Dev,
        })
        .expect("offline client construction")
    }

    const PARAM_SCRIPT: &str = r#"
// @parameters: [
//   { "id": "threshold", "type": "number", "default": 30.0, "min": 10.0, "max": 90.0 }
// ]
fn on_candle() {
    "hold"
}
"#;

    fn definition(strategy_type: &str, script: Option<&str>) -> StrategyDefinition {
        StrategyDefinition {
            id: "test-strategy".to_string(),
            user_id: "test-user".to_string(),
            name: "test".to_string(),
            description: "test".to_string(),
            parameters: vec![],
            indicators: vec![],
            variables: vec![],
            entry_rules: vec![],
            entry_logic: EntryLogic {
                mode: EntryLogicMode::All,
                min_score: None,
            },
            exit_rules: vec![],
            risk_settings: RiskSettings {
                risk_method: RiskMethod::Percent,
                risk_value: ParameterizedValue::Fixed(1.0),
                rr_ratio: ParameterizedValue::Fixed(2.0),
                spread_buffer_pips: ParameterizedValue::Fixed(1.0),
                stop_loss_source: None,
                risk_method_short: None,
                risk_value_short: None,
                rr_ratio_short: None,
                spread_buffer_pips_short: None,
                stop_loss_source_short: None,
            },
            version: 1,
            is_active: true,
            schema_version: 2,
            strategy_type: strategy_type.to_string(),
            script_content: script.map(|s| s.to_string()),
        }
    }

    fn watcher(strategy: StrategyDefinition) -> MultiInstrumentWatcher {
        let (_tx, rx) = mpsc::channel(1);
        MultiInstrumentWatcher::new(
            "test-watcher".to_string(),
            "test-strategy".to_string(),
            "test".to_string(),
            strategy,
            Granularity::H1,
            "test-user".to_string(),
            dummy_client(),
            ExecutionMode::SignalOnly,
            rx,
            Arc::new(AtomicBool::new(false)),
        )
    }

    // AC2: `--set` overrides handed to the watcher reach every
    // per-instrument scripted-strategy instance.
    #[tokio::test]
    async fn script_params_flow_into_each_instrument_executor() {
        let mut w = watcher(definition("scripted", Some(PARAM_SCRIPT)));
        w.set_script_params(HashMap::from([("threshold".to_string(), 55.0)]));
        w.add_instrument_with_source(
            "EUR_USD".to_string(),
            Box::new(NoopCandleSource),
            Vec::new(),
            "all".to_string(),
        )
        .await
        .unwrap();

        let state = w.instruments.get("EUR_USD").unwrap();
        match &state.executor {
            StrategyExecutor::Scripted(s) => {
                assert_eq!(s.get_resolved_params().get("threshold"), Some(&55.0));
            }
            _ => panic!("expected a scripted executor"),
        }
    }

    // ABI v4: the surprise calendar handed to the watcher reaches every
    // per-instrument scripted-strategy instance, with the instrument's legs
    // as the default currency filter — live watch parity with backtest.
    #[tokio::test]
    async fn script_surprise_calendar_flows_into_each_instrument_executor() {
        use crate::backtest::surprise::FF_CSV_HEADER;

        // A "USD CPI" series with 8 discovery surprises of ±1 (mean 0,
        // pstdev 1) and one later release with surprise +2 → z = +2.
        let mut csv = format!("{FF_CSV_HEADER}\n");
        for i in 0..8 {
            let actual = if i % 2 == 0 { 4.0 } else { 2.0 };
            csv.push_str(&format!(
                "2024-0{}-10,13:30,USD,CPI y/y,high,{actual}%,3.0%,3.0%\n",
                i + 1
            ));
        }
        csv.push_str("2024-09-01,10:00,USD,CPI y/y,high,5.0%,3.0%,3.1%\n");
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("cal.csv"), csv).unwrap();
        let calendar = SurpriseCalendar::load_dir(dir.path()).unwrap();

        const SURPRISE_SCRIPT: &str = r#"
fn on_candle() {
    if surprise_z() > 1.5 && surprise_hours_ago() < 24.0 { "buy" } else { "hold" }
}
"#;
        let mut w = watcher(definition("scripted", Some(SURPRISE_SCRIPT)));
        w.set_script_surprise_calendar(calendar);
        w.add_instrument_with_source(
            "EUR_USD".to_string(),
            Box::new(NoopCandleSource),
            Vec::new(),
            "all".to_string(),
        )
        .await
        .unwrap();

        let state = w.instruments.get_mut("EUR_USD").unwrap();
        let mut candle = test_candle();
        candle.time = chrono::DateTime::parse_from_rfc3339("2024-09-01T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        match &mut state.executor {
            StrategyExecutor::Scripted(s) => {
                use crate::backtest::Strategy;
                assert_eq!(s.on_candle(&candle), crate::backtest::Signal::Buy);
            }
            _ => panic!("expected a scripted executor"),
        }
    }

    // Without overrides, the script's declared defaults apply — parity with
    // `ScriptedStrategy::from_script`.
    #[tokio::test]
    async fn scripted_instruments_default_to_declared_parameters() {
        let mut w = watcher(definition("scripted", Some(PARAM_SCRIPT)));
        w.add_instrument_with_source(
            "EUR_USD".to_string(),
            Box::new(NoopCandleSource),
            Vec::new(),
            "all".to_string(),
        )
        .await
        .unwrap();

        let state = w.instruments.get("EUR_USD").unwrap();
        match &state.executor {
            StrategyExecutor::Scripted(s) => {
                assert_eq!(s.get_resolved_params().get("threshold"), Some(&30.0));
            }
            _ => panic!("expected a scripted executor"),
        }
    }

    // AC3: a scripted entry's signal-map fields (stop_loss/take_profit/
    // rule_name) survive into the emitted signal. Uses the scripted sizing
    // short-circuit, so no network is touched.
    #[tokio::test]
    async fn scripted_entry_signal_keeps_sl_tp_and_rule_name() {
        use rust_decimal_macros::dec;
        let w = watcher(definition("scripted", Some(PARAM_SCRIPT)));
        let candle = test_candle();
        let signal = RulesSignal::Entry {
            direction: PositionDirection::Long,
            stop_loss: Some(dec!(1.0800)),
            take_profit: Some(dec!(1.0950)),
            triggered_rule_id: Some("adx_gate".to_string()),
            triggered_rule_name: Some("adx_gate".to_string()),
            pending_order: None,
        };

        let out = w
            .create_signal(&signal, None, &candle, None, "EUR_USD")
            .await
            .expect("entry signal emitted");
        assert_eq!(out.match_type, SignalType::Entry);
        assert_eq!(out.stop_loss, Some(dec!(1.0800)));
        assert_eq!(out.take_profit, Some(dec!(1.0950)));
        assert_eq!(out.rule_name.as_deref(), Some("adx_gate"));
        // Scripted sizing is the script's business — the watcher proposes none.
        assert_eq!(out.position_size, None);
    }

    // Regression: a scripted entry that emits NO stop_loss/take_profit (e.g.
    // revert_adx, which exits on the opposite signal) must yield None brackets,
    // not SL==TP==entry_price. The degenerate default produced OANDA
    // TAKE_PROFIT_ON_FILL_LOSS rejections that silently bricked --auto watchers.
    #[tokio::test]
    async fn scripted_entry_without_sl_tp_emits_no_bracket() {
        let w = watcher(definition("scripted", Some(PARAM_SCRIPT)));
        let candle = test_candle();
        let signal = RulesSignal::Entry {
            direction: PositionDirection::Long,
            stop_loss: None,
            take_profit: None,
            triggered_rule_id: Some("adx_gate".to_string()),
            triggered_rule_name: Some("adx_gate".to_string()),
            pending_order: None,
        };

        let out = w
            .create_signal(&signal, None, &candle, None, "EUR_USD")
            .await
            .expect("entry signal emitted");
        assert_eq!(out.match_type, SignalType::Entry);
        // The bug: these were Some(entry_price). Now they stay None so the
        // order carries no on-fill bracket and OANDA accepts it.
        assert_eq!(out.stop_loss, None);
        assert_eq!(out.take_profit, None);
    }

    // AC3: a scripted close signal fires even with no broker position (the
    // script tracks its position virtually) — direction unknown, exit_reason
    // preserved as the signal's reason.
    #[tokio::test]
    async fn scripted_exit_signal_survives_without_a_broker_position() {
        let w = watcher(definition("scripted", Some(PARAM_SCRIPT)));
        let candle = test_candle();
        let signal = RulesSignal::Exit {
            reason: "adx recovered".to_string(),
            close_percent: 1.0,
        };

        let out = w
            .create_signal(&signal, None, &candle, None, "EUR_USD")
            .await
            .expect("scripted exit must surface");
        assert_eq!(out.match_type, SignalType::Exit);
        assert_eq!(out.direction, None);
        assert_eq!(out.reason, "adx recovered");
    }

    // AC4 guard: rules-based strategies keep the existing behavior — an exit
    // with no open position is dropped, exactly as before AGT-624.
    #[tokio::test]
    async fn rules_exit_signal_is_still_dropped_when_flat() {
        let w = watcher(definition("rules", None));
        let candle = test_candle();
        let signal = RulesSignal::Exit {
            reason: "take profit".to_string(),
            close_percent: 1.0,
        };

        assert!(w.create_signal(&signal, None, &candle, None, "EUR_USD").await.is_none());
    }

    fn test_candle() -> Candle {
        use rust_decimal_macros::dec;
        Candle {
            time: chrono::Utc::now(),
            volume: 100,
            complete: true,
            mid: crate::models::Ohlc {
                open: dec!(1.0850),
                high: dec!(1.0860),
                low: dec!(1.0840),
                close: dec!(1.0855),
            },
        }
    }

    // ---- restart backfill (watch-state ledger) ----

    fn candle_at(rfc3339: &str) -> Candle {
        let mut c = test_candle();
        c.time = DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .with_timezone(&Utc);
        c
    }

    /// Offline source with a fixed history — lets warmup tests control
    /// exactly which candles exist.
    struct FixedHistorySource {
        history: Vec<Candle>,
    }

    impl CandleSource for FixedHistorySource {
        fn get_latest_candle(
            &self,
        ) -> Pin<Box<dyn Future<Output = crate::error::Result<Option<Candle>>> + Send + '_>>
        {
            Box::pin(async { Ok(None) })
        }

        fn get_candles(
            &self,
            _count: u32,
        ) -> Pin<Box<dyn Future<Output = crate::error::Result<Vec<Candle>>> + Send + '_>> {
            let history = self.history.clone();
            Box::pin(async move { Ok(history) })
        }

        fn timeframe(&self) -> Granularity {
            Granularity::H1
        }

        fn instrument(&self) -> &str {
            "EUR_USD"
        }

        fn poll_interval(&self) -> Duration {
            Duration::from_secs(1)
        }
    }

    /// Sink that records every event category the backfill touches.
    #[derive(Default)]
    struct RecordingSink {
        ticks: std::sync::Mutex<Vec<WatcherTickEvent>>,
        errors: std::sync::Mutex<Vec<StrategyErrorEvent>>,
        signals: std::sync::Mutex<Vec<StrategySignalEvent>>,
    }

    impl EventSink for RecordingSink {
        fn pattern_matched(&self, event: &StrategySignalEvent) {
            self.signals.lock().unwrap().push(event.clone());
        }
        fn strategy_status(&self, _event: &StrategyStatusEvent) {}
        fn strategy_error(&self, event: &StrategyErrorEvent) {
            self.errors.lock().unwrap().push(event.clone());
        }
        fn match_status_update(&self, _event: &SignalStatusUpdateEvent) {}
        fn watcher_tick(&self, event: &WatcherTickEvent) {
            self.ticks.lock().unwrap().push(event.clone());
        }
        fn price_update(&self, _event: &crate::oanda::streaming::PriceUpdate) {}
        fn stream_error(&self, _event: &crate::oanda::streaming::StreamError) {}
        fn stream_health(&self, _event: &crate::oanda::streaming::StreamHealthStatus) {}
    }

    const ALWAYS_BUY_SCRIPT: &str = r#"
fn on_candle() {
    "buy"
}
"#;

    fn hourly_candles(n: usize) -> Vec<Candle> {
        (0..n)
            .map(|i| candle_at(&format!("2026-07-14T{:02}:00:00Z", i)))
            .collect()
    }

    async fn instrument_state_with_history(history: Vec<Candle>) -> InstrumentState {
        InstrumentState::new(
            "EUR_USD".to_string(),
            Box::new(FixedHistorySource { history }),
            &definition("scripted", Some(ALWAYS_BUY_SCRIPT)),
            Vec::new(),
            "all".to_string(),
            "H1",
            &HashMap::new(),
            None,
            None,
        )
        .unwrap()
    }

    // No ledger entry (fresh start): everything is warmup, nothing to replay —
    // the pre-ledger behavior, unchanged.
    #[tokio::test]
    async fn warmup_without_cutoff_stashes_no_backfill() {
        let mut state = instrument_state_with_history(hourly_candles(6)).await;
        let covered = state.warmup(100, None).await.unwrap();
        assert!(state.pending_backfill.is_empty());
        assert_eq!(covered, Some(candle_at("2026-07-14T05:00:00Z").time));
    }

    // With a ledger cutoff, candles after it are stashed for replay and NOT
    // fed to warmup — each candle must advance the executor exactly once.
    #[tokio::test]
    async fn warmup_splits_history_at_the_ledger_cutoff() {
        let mut state = instrument_state_with_history(hourly_candles(6)).await;
        let cutoff = candle_at("2026-07-14T03:00:00Z").time;
        let covered = state.warmup(100, Some(cutoff)).await.unwrap();

        let stashed: Vec<_> = state.pending_backfill.iter().map(|c| c.time).collect();
        assert_eq!(
            stashed,
            vec![
                candle_at("2026-07-14T04:00:00Z").time,
                candle_at("2026-07-14T05:00:00Z").time,
            ]
        );
        // Warmup covered exactly the candles up to the cutoff.
        assert_eq!(covered, Some(cutoff));
    }

    // A gap longer than the replay cap folds its oldest candles back into
    // warmup; only the newest MAX_BACKFILL_CANDLES are replayed.
    #[tokio::test]
    async fn warmup_caps_the_replay_at_max_backfill_candles() {
        let n = MAX_BACKFILL_CANDLES + 10;
        // 1 candle before the cutoff + n after it.
        let mut history = vec![candle_at("2026-07-10T00:00:00Z")];
        history.extend((0..n).map(|i| {
            candle_at(&format!("2026-07-14T{:02}:{:02}:00Z", i / 60, i % 60))
        }));
        let mut state = instrument_state_with_history(history).await;

        let cutoff = candle_at("2026-07-10T00:00:00Z").time;
        state.warmup(100, Some(cutoff)).await.unwrap();

        assert_eq!(state.pending_backfill.len(), MAX_BACKFILL_CANDLES);
        // The newest survive; the oldest of the gap were folded into warmup.
        assert_eq!(
            state.pending_backfill.last().unwrap().time,
            candle_at(&format!("2026-07-14T00:{:02}:00Z", n - 1)).time
        );
    }

    // The replay policy: every candle emits a backfill tick; interior Entry
    // signals are reported as missed (never tradeable); the newest candle's
    // Entry emits normally, exactly like the regular initial evaluation.
    #[tokio::test]
    async fn backfill_suppresses_interior_entries_and_emits_the_newest() {
        let mut w = watcher(definition("scripted", Some(ALWAYS_BUY_SCRIPT)));
        w.add_instrument_with_source(
            "EUR_USD".to_string(),
            Box::new(NoopCandleSource),
            Vec::new(),
            "all".to_string(),
        )
        .await
        .unwrap();

        let replay = vec![
            candle_at("2026-07-15T02:00:00Z"),
            candle_at("2026-07-15T10:00:00Z"),
            candle_at("2026-07-15T18:00:00Z"),
        ];
        w.instruments.get_mut("EUR_USD").unwrap().pending_backfill = replay;
        // Seed the position cache so the replay never touches the network.
        w.cached_positions = Some((Instant::now(), HashMap::new()));

        let sink = RecordingSink::default();
        w.backfill_instrument(&sink, "EUR_USD").await.unwrap();

        let ticks = sink.ticks.lock().unwrap();
        assert_eq!(ticks.len(), 3, "every replayed candle gets a tick");
        assert!(ticks.iter().all(|t| t.backfill), "replay ticks are marked");
        assert!(
            ticks.iter().all(|t| t.signal_result == "Entry Long"),
            "the always-buy script fires on every candle"
        );

        let errors = sink.errors.lock().unwrap();
        let missed: Vec<_> = errors.iter().filter(|e| e.error_type == "missed_signal").collect();
        assert_eq!(missed.len(), 2, "both interior entries are reported as missed");
        assert!(missed.iter().all(|e| e.message.contains("not emitted as tradeable")));

        let signals = sink.signals.lock().unwrap();
        assert_eq!(signals.len(), 1, "only the newest candle's entry is tradeable");
        assert_eq!(
            signals[0].pattern_match.instrument, "EUR_USD",
            "emitted signal belongs to the replayed instrument"
        );
    }

    // The ledger advances through the replay, so a crash mid-backfill (or a
    // clean run) resumes from the last candle actually evaluated.
    #[tokio::test]
    async fn backfill_records_progress_in_the_ledger() {
        use super::super::watch_state::WatchStateStore;

        let dir = tempfile::tempdir().unwrap();
        let mut w = watcher(definition("scripted", Some(ALWAYS_BUY_SCRIPT)));
        w.set_state_store(WatchStateStore::open(dir.path(), "test-watcher").unwrap());
        w.add_instrument_with_source(
            "EUR_USD".to_string(),
            Box::new(NoopCandleSource),
            Vec::new(),
            "all".to_string(),
        )
        .await
        .unwrap();

        let newest = candle_at("2026-07-15T18:00:00Z").time;
        w.instruments.get_mut("EUR_USD").unwrap().pending_backfill = vec![
            candle_at("2026-07-15T10:00:00Z"),
            candle_at("2026-07-15T18:00:00Z"),
        ];
        w.cached_positions = Some((Instant::now(), HashMap::new()));

        let sink = RecordingSink::default();
        w.backfill_instrument(&sink, "EUR_USD").await.unwrap();

        let reopened = WatchStateStore::open(dir.path(), "test-watcher").unwrap();
        assert_eq!(reopened.last_evaluated("EUR_USD"), Some(newest));
    }
}
