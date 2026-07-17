//! Rhai Scripted Strategy
//!
//! Implements the Strategy trait via an embedded Rhai scripting engine.
//! Scripts declare indicators and parameters in structured metadata comments,
//! and implement an `on_candle()` function that returns trading signals.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rhai::{Dynamic, Engine, EvalAltResult, Scope, AST, Map as RhaiMap};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tracing::warn;

use crate::models::Candle;
use shared::{EntryOrderType, IndicatorConfig, IndicatorType, ParameterDefinition, ParameterType, ParameterizedValue, ParameterOption};

use super::indicator_engine::IndicatorEngine;
use super::strategy::{ExtendedSignal, PendingOrderInfo, PositionSnapshot, Signal, Strategy};
use super::surprise::{self, SurpriseCalendar, SurpriseRelease};

// Maximum number of historical indicator values to track for lookback
const INDICATOR_LOOKBACK: usize = 10;
// Maximum number of price candles to expose to scripts
const MAX_PRICE_HISTORY: usize = 100;
// Maximum consecutive Rhai errors before aborting
const MAX_CONSECUTIVE_ERRORS: usize = 50;

// =============================================================================
// Resource-safety limits (AGT-606)
// =============================================================================
//
// `configure_engine_limits` is the single place every Rhai `Engine` this module
// constructs — both the runtime execution path (`from_script*`, `from_precompiled`,
// `precompile`) and the validate-time path (`validate_script`) — gets its sandbox
// limits from. Before AGT-606, `validate_script`/`precompile` set three of the four
// limits (missing `max_call_levels`) while the runtime path set four, so a script
// could pass validation with recursion depth the runtime would actually reject (or
// vice versa). Calling this one function from every constructor site means the two
// paths can never drift apart again.

/// Maximum length of any array a script creates (literal, `push`, etc.).
const MAX_ARRAY_SIZE: usize = 10_000;
/// Maximum size of any object map a script creates (`#{ ... }`, `push`, etc.).
const MAX_MAP_SIZE: usize = 1_000;

/// Wall-clock budget for a single script entry-point call (`on_candle()`,
/// `on_position_closed()`, or the script's top-level init code). This is the
/// daemon's real anti-hang guard: `max_operations` bounds *work*, but a script can
/// spend its operation budget on something that's individually slow (e.g. building
/// huge strings) and still blow well past what a live watcher can tolerate between
/// candles. Checked via `on_progress`, which fires periodically during execution —
/// see `register_wall_clock_guard`.
const ON_CANDLE_WALL_CLOCK_BUDGET: Duration = Duration::from_millis(50);

/// How often (in Rhai operations) the wall-clock guard actually checks
/// `Instant::now()`. Checking on every single operation would add measurable
/// overhead to every script call; sampling every N operations keeps that overhead
/// negligible while still aborting well within budget for a genuinely runaway script.
const WALL_CLOCK_CHECK_INTERVAL_OPS: u64 = 64;

/// Configure the sandbox limits shared by every Rhai engine this module constructs.
/// See the module-level comment above — this is the one call site that keeps
/// validate-time and run-time limits from drifting apart.
fn configure_engine_limits(engine: &mut Engine) {
    engine.set_max_operations(1_000_000);
    engine.set_max_call_levels(32);
    engine.set_max_expr_depths(64, 64);
    engine.set_max_string_size(10_000);
    engine.set_max_array_size(MAX_ARRAY_SIZE);
    engine.set_max_map_size(MAX_MAP_SIZE);
}

/// Register the wall-clock anti-hang guard on `engine` via Rhai's `on_progress` hook,
/// and return the shared start-time handle. Callers must reset the handle to
/// `Instant::now()` immediately before invoking any script entry point (the same
/// engine/AST runs multiple entry points over its lifetime — init code, every
/// `on_candle()` call, `on_position_closed()` — so the budget applies per-call, not
/// cumulatively).
///
/// Only used by the runtime execution path. `validate_script` and `precompile` never
/// execute script bodies (compilation is static analysis only), so they have nothing
/// to bound here.
fn register_wall_clock_guard(engine: &mut Engine) -> Arc<Mutex<Instant>> {
    let start = Arc::new(Mutex::new(Instant::now()));
    let start_for_progress = Arc::clone(&start);

    engine.on_progress(move |ops| {
        if ops % WALL_CLOCK_CHECK_INTERVAL_OPS != 0 {
            return None;
        }
        let elapsed = start_for_progress
            .lock()
            .map(|s| s.elapsed())
            .unwrap_or_default();
        if elapsed > ON_CANDLE_WALL_CLOCK_BUDGET {
            Some(Dynamic::from(format!(
                "script exceeded {}ms wall-clock budget (ran ~{}ms)",
                ON_CANDLE_WALL_CLOCK_BUDGET.as_millis(),
                elapsed.as_millis(),
            )))
        } else {
            None
        }
    });

    start
}

/// Reset a wall-clock guard's start time to now. Callers must do this immediately
/// before invoking any script entry point — see `register_wall_clock_guard`.
fn reset_wall_clock(start: &Arc<Mutex<Instant>>) {
    if let Ok(mut s) = start.lock() {
        *s = Instant::now();
    }
}

/// If a Rhai script error was the wall-clock guard terminating the script (rather
/// than an ordinary script error), extract the guard's message. `ErrorTerminated`
/// carries its payload as a `Dynamic`, and `EvalAltResult`'s `Display` impl
/// deliberately renders it as just "Script terminated" without the payload — so
/// callers that want the actual reason (for logging) have to unpack it themselves.
fn wall_clock_terminated_reason(err: &EvalAltResult) -> Option<String> {
    match err {
        EvalAltResult::ErrorTerminated(token, _) => token.clone().into_string().ok(),
        _ => None,
    }
}

// =============================================================================
// Shared script context — updated before each on_candle call, read by SDK fns
// =============================================================================

/// Holds the data that Rhai SDK functions need access to.
/// Stored behind `Arc<Mutex<>>` so registered native functions can read it.
#[derive(Debug, Clone, Default)]
struct ScriptContext {
    candle: RhaiMap,
    indicators: RhaiMap,
    indicator_history: RhaiMap,
    prices: rhai::Array,
    params: RhaiMap,
    bar_count: i64,
    volume: i64,
    pip_value: Decimal,
    /// Current candle's open time as Unix seconds (UTC). Staged per candle;
    /// 0 until the first candle arrives (ABI v2: `candle_time()`).
    candle_time_unix: i64,
    /// Current candle's open hour, 0–23 UTC (ABI v2: `candle_hour()`).
    candle_hour: i64,
    /// Hours since the most recent calendar event at or before this candle's
    /// open; -1 when no calendar is set or no prior event exists (ABI v3:
    /// `hours_since_event()`).
    hours_since_event: Decimal,
    /// Hours until the next calendar event strictly after this candle's open;
    /// -1 when no calendar is set or no later event is known (ABI v3:
    /// `hours_until_event()`).
    hours_until_event: Decimal,
    /// Scored surprise releases (sorted by time), feeding the `surprise_z()`
    /// family (ABI v4). The `Arc` snapshot is swapped by the host when the
    /// calendar directory changes; empty = no feed (accessors return their
    /// sentinels).
    surprise_releases: Arc<Vec<SurpriseRelease>>,
    /// The instrument's currency legs (e.g. ["EUR", "USD"]) — the default
    /// currency filter for the `surprise_z()` family, matching the ABI v3
    /// event-calendar leg filter.
    surprise_legs: Vec<String>,
    /// ABI v5 position state, staged per candle from the engine's
    /// `sync_position_state` push. `in_position()` reads this directly;
    /// when flat, `entry_price` is 0 and `bars_since_entry` is -1.
    in_position: bool,
    entry_price: Decimal,
    bars_since_entry: i64,
}

/// Metadata extracted from a Rhai script's structured comments.
#[derive(Debug, Clone)]
pub struct ScriptMetadata {
    pub indicators: Vec<IndicatorConfig>,
    pub parameters: Vec<ParameterDefinition>,
}

/// A strategy backed by a Rhai script.
pub struct ScriptedStrategy {
    name: String,
    engine: Engine,
    ast: AST,
    scope: Scope<'static>,
    indicator_engine: IndicatorEngine,
    indicator_configs: Vec<IndicatorConfig>,
    parameters: Vec<ParameterDefinition>,
    resolved_params: HashMap<String, f64>,
    price_history: VecDeque<Candle>,
    max_price_history: usize,
    last_stop_loss: Option<Decimal>,
    last_take_profit: Option<Decimal>,
    last_exit_reason: Option<String>,
    last_entry_rule_name: Option<String>,
    bar_count: usize,
    consecutive_errors: usize,
    /// `true` once a `strategy_health` "aborted" event has been emitted for the
    /// current run of consecutive errors — cleared on the next successful call (or
    /// on `reset()`) so a later abort can be reported again. See `take_abort_event`.
    abort_event_emitted: bool,
    /// Economic-calendar event times (Unix seconds, sorted ascending) that
    /// feed `hours_since_event()`/`hours_until_event()`. Empty = no calendar
    /// (the accessors return -1). Injected by the host via
    /// [`Self::set_event_calendar`]; both the backtest engine and the live
    /// watcher run through `update_context`, so one injection serves both.
    event_times: Vec<i64>,
    /// Updatable surprise calendar feeding the `surprise_z()` family (ABI
    /// v4). Injected via [`Self::set_surprise_calendar`]; `None` = no feed.
    /// Refreshed from the candle path so a long-lived watcher sees CSV drops
    /// (including backfilled actuals) without a restart.
    surprise: Option<SurpriseCalendar>,
    /// Currency legs staged into the context alongside the surprise feed.
    surprise_legs: Vec<String>,
    /// The engine's open-position snapshot (ABI v5), pushed once per candle
    /// via [`Strategy::sync_position_state`]. `None` = flat, or a host that
    /// never simulates positions (live watcher / `strategy run`) — the
    /// script accessors then return their flat sentinels.
    position: Option<PositionSnapshot>,
    /// Completed candles since the current position opened (0 on the entry
    /// candle). Only meaningful while `position.is_some()`.
    bars_in_position: i64,
    /// Shared context readable by registered native SDK functions.
    ctx: Arc<Mutex<ScriptContext>>,
    /// Wall-clock budget start time, shared with the `on_progress` guard. Reset
    /// immediately before every script entry-point call.
    wall_clock_start: Arc<Mutex<Instant>>,
}

impl ScriptedStrategy {
    /// Create a scripted strategy from source code, using default parameter values.
    pub fn from_script(script: &str, name: &str) -> Result<Self, String> {
        Self::from_script_with_params(script, name, HashMap::new())
    }

    /// One-stop host constructor (AGT-651): build a scripted strategy the way
    /// EVERY host must — parameters resolved (defaults + overrides), pip
    /// value set for the instrument, and both calendars (event ABI v3 +
    /// surprise ABI v4) injected from the wickd data home. The CLI and the
    /// desktop app both construct through here, so host wiring can no longer
    /// drift (dialect report D1/D2/D3 — the app used to skip the calendar
    /// setters, silently pinning `hours_since_event()` at -1 and
    /// `surprise_z()` at -9999).
    ///
    /// Calendar sources: `~/.wickd/events.json` (else the bundled schedule)
    /// and `~/.wickd/calendar/*.csv` — see [`crate::events`]. A missing
    /// surprise dir is an empty feed; malformed calendar files are hard
    /// errors, matching the CLI's long-standing behavior.
    pub fn for_host(
        script: &str,
        name: &str,
        param_overrides: HashMap<String, f64>,
        instrument: &str,
    ) -> Result<Self, String> {
        let mut strategy = Self::from_script_with_params(script, name, param_overrides)?;
        strategy.set_pip_value_for_instrument(instrument);
        let (events, _source) = crate::events::load_for_instrument(instrument)?;
        strategy.set_event_calendar(events);
        let surprise = crate::events::load_surprise_calendar()?;
        strategy.set_surprise_calendar(surprise, instrument);
        Ok(strategy)
    }

    /// Create a scripted strategy from source code with parameter overrides.
    pub fn from_script_with_params(
        script: &str,
        name: &str,
        param_overrides: HashMap<String, f64>,
    ) -> Result<Self, String> {
        let metadata = parse_metadata(script)?;

        // Build resolved params: defaults first, then overrides
        let mut resolved_params = HashMap::new();
        for p in &metadata.parameters {
            resolved_params.insert(p.id.clone(), p.default);
        }
        for (k, v) in &param_overrides {
            if resolved_params.contains_key(k) {
                resolved_params.insert(k.clone(), *v);
            }
        }

        // Build indicator engine
        let indicator_engine = IndicatorEngine::from_config_with_params(
            &metadata.indicators,
            INDICATOR_LOOKBACK,
            &resolved_params,
        )?;

        // Create the shared context
        let ctx = Arc::new(Mutex::new(ScriptContext::default()));

        // Build Rhai engine with native SDK functions
        let mut engine = Engine::new();
        configure_engine_limits(&mut engine);
        let wall_clock_start = register_wall_clock_guard(&mut engine);

        register_sdk_functions(&mut engine, Arc::clone(&ctx));

        // Compile the user script
        let ast = engine
            .compile(script)
            .map_err(|e| format!("Script compilation error: {}", e))?;

        // Verify on_candle function exists
        verify_on_candle_exists(&ast)?;

        // Initialize scope with top-level script execution (sets up any global vars)
        let mut scope = Scope::new();

        // Run top-level code to initialize user-defined state variables
        reset_wall_clock(&wall_clock_start);
        engine
            .run_ast_with_scope(&mut scope, &ast)
            .map_err(|e| format!("Script initialization error: {}", e))?;

        Ok(Self {
            name: name.to_string(),
            engine,
            ast,
            scope,
            indicator_engine,
            indicator_configs: metadata.indicators,
            parameters: metadata.parameters,
            resolved_params,
            price_history: VecDeque::new(),
            max_price_history: MAX_PRICE_HISTORY,
            last_stop_loss: None,
            last_take_profit: None,
            last_exit_reason: None,
            last_entry_rule_name: None,
            bar_count: 0,
            consecutive_errors: 0,
            abort_event_emitted: false,
            event_times: Vec::new(),
            surprise: None,
            surprise_legs: Vec::new(),
            position: None,
            bars_in_position: 0,
            ctx,
            wall_clock_start,
        })
    }

    /// Create a scripted strategy from pre-parsed metadata and pre-compiled AST.
    ///
    /// This avoids re-parsing metadata and re-compiling the script on every call,
    /// which is critical for optimization where thousands of parameter combinations
    /// are tested against the same script.
    pub fn from_precompiled(
        metadata: &ScriptMetadata,
        ast: &AST,
        name: &str,
        param_overrides: HashMap<String, f64>,
    ) -> Result<Self, String> {
        // Build resolved params: defaults first, then overrides
        let mut resolved_params = HashMap::new();
        for p in &metadata.parameters {
            resolved_params.insert(p.id.clone(), p.default);
        }
        for (k, v) in &param_overrides {
            if resolved_params.contains_key(k) {
                resolved_params.insert(k.clone(), *v);
            }
        }

        // Build indicator engine
        let indicator_engine = IndicatorEngine::from_config_with_params(
            &metadata.indicators,
            INDICATOR_LOOKBACK,
            &resolved_params,
        )?;

        // Create the shared context
        let ctx = Arc::new(Mutex::new(ScriptContext::default()));

        // Build Rhai engine with native SDK functions (engine is cheap, AST compile is expensive)
        let mut engine = Engine::new();
        configure_engine_limits(&mut engine);
        let wall_clock_start = register_wall_clock_guard(&mut engine);

        register_sdk_functions(&mut engine, Arc::clone(&ctx));

        // Clone the pre-compiled AST (cheap — AST is reference-counted internally)
        let ast = ast.clone();

        // Initialize scope with top-level script execution
        let mut scope = Scope::new();
        reset_wall_clock(&wall_clock_start);
        engine
            .run_ast_with_scope(&mut scope, &ast)
            .map_err(|e| format!("Script initialization error: {}", e))?;

        Ok(Self {
            name: name.to_string(),
            engine,
            ast,
            scope,
            indicator_engine,
            indicator_configs: metadata.indicators.clone(),
            parameters: metadata.parameters.clone(),
            resolved_params,
            price_history: VecDeque::new(),
            max_price_history: MAX_PRICE_HISTORY,
            last_stop_loss: None,
            last_take_profit: None,
            last_exit_reason: None,
            last_entry_rule_name: None,
            bar_count: 0,
            consecutive_errors: 0,
            abort_event_emitted: false,
            event_times: Vec::new(),
            surprise: None,
            surprise_legs: Vec::new(),
            position: None,
            bars_in_position: 0,
            ctx,
            wall_clock_start,
        })
    }

    /// Pre-compile a script for reuse with `from_precompiled`.
    /// Returns the metadata and compiled AST.
    pub fn precompile(script: &str) -> Result<(ScriptMetadata, AST), String> {
        let metadata = parse_metadata(script)?;

        let ctx = Arc::new(Mutex::new(ScriptContext::default()));
        let mut engine = Engine::new();
        configure_engine_limits(&mut engine);
        register_sdk_functions(&mut engine, ctx);

        let ast = engine
            .compile(script)
            .map_err(|e| format!("Script compilation error: {}", e))?;

        verify_on_candle_exists(&ast)?;

        Ok((metadata, ast))
    }

    /// Get the parameter definitions declared by this script.
    pub fn get_parameters(&self) -> &[ParameterDefinition] {
        &self.parameters
    }

    /// Get the indicator configurations declared by this script.
    pub fn get_indicator_configs(&self) -> &[IndicatorConfig] {
        &self.indicator_configs
    }

    /// Get the resolved parameter values (defaults + overrides).
    pub fn get_resolved_params(&self) -> &HashMap<String, f64> {
        &self.resolved_params
    }

    /// Set pip value for the instrument in the shared context.
    pub fn set_pip_value_for_instrument(&mut self, instrument: &str) {
        let pip = pip_value_for_instrument(instrument);
        if let Ok(mut ctx) = self.ctx.lock() {
            ctx.pip_value = pip;
        }
    }

    /// Inject the economic-calendar event times feeding
    /// `hours_since_event()`/`hours_until_event()` (ABI v3). Sorted and
    /// deduplicated here so `update_context` can binary-search per candle.
    /// An empty calendar (or never calling this) leaves both accessors at -1.
    pub fn set_event_calendar(&mut self, times: Vec<chrono::DateTime<chrono::Utc>>) {
        let mut unix: Vec<i64> = times.into_iter().map(|t| t.timestamp()).collect();
        unix.sort_unstable();
        unix.dedup();
        self.event_times = unix;
    }

    /// Inject the updatable surprise calendar feeding the `surprise_z()` /
    /// `surprise_hours_ago()` family (ABI v4), and record the instrument's
    /// currency legs as the accessors' default currency filter. Never calling
    /// this leaves the accessors at their sentinels. The calendar is
    /// re-checked for CSV drops from the candle path (see
    /// [`SurpriseCalendar::maybe_refresh`]), so one injection also serves a
    /// long-lived watcher.
    pub fn set_surprise_calendar(&mut self, calendar: SurpriseCalendar, instrument: &str) {
        self.surprise_legs = instrument.split('_').map(str::to_string).collect();
        if let Ok(mut ctx) = self.ctx.lock() {
            ctx.surprise_releases = calendar.releases();
            ctx.surprise_legs = self.surprise_legs.clone();
        }
        self.surprise = Some(calendar);
    }

    /// Get a snapshot of current indicator values.
    pub fn get_indicator_snapshot(&self) -> HashMap<String, HashMap<String, String>> {
        self.indicator_engine.get_snapshot()
    }

    /// Edge-triggered: returns `true` exactly once, on the first `on_candle_extended`
    /// call after the consecutive-error abort threshold trips, so callers can emit a
    /// `strategy_health`/"aborted" event distinguishable from the `Hold` the strategy
    /// returns on every subsequent candle. Before AGT-606 that `Hold` was
    /// indistinguishable from a strategy legitimately finding no signal — a live
    /// watcher had no way to tell "the script is broken" from "nothing to do here".
    ///
    /// Returns `false` on every call after the first, until either the strategy
    /// recovers (a successful `on_candle()` call) or `reset()` is called, either of
    /// which re-arms the trigger for a future abort.
    pub fn take_abort_event(&mut self) -> bool {
        if self.consecutive_errors >= MAX_CONSECUTIVE_ERRORS && !self.abort_event_emitted {
            self.abort_event_emitted = true;
            true
        } else {
            false
        }
    }

    /// Human-readable reason for the abort event `take_abort_event` reports —
    /// suitable as an event/log message.
    pub fn abort_reason(&self) -> String {
        format!(
            "Rhai script '{}' aborted after {MAX_CONSECUTIVE_ERRORS} consecutive on_candle() \
             errors — it is now returning Hold on every candle because on_candle() keeps \
             failing, not because it found no signal. Fix the script and restart the watcher.",
            self.name,
        )
    }

    /// Update the shared context with the current candle, indicator values, etc.
    fn update_context(&self, candle: &Candle) {
        let mut ctx = self.ctx.lock().expect("Script context lock poisoned");

        // __candle map
        let mut candle_map = RhaiMap::new();
        candle_map.insert("open".into(), Dynamic::from(candle.mid.open));
        candle_map.insert("high".into(), Dynamic::from(candle.mid.high));
        candle_map.insert("low".into(), Dynamic::from(candle.mid.low));
        candle_map.insert("close".into(), Dynamic::from(candle.mid.close));
        ctx.candle = candle_map;

        // Candle clock (ABI v2): open time as Unix seconds + 0–23 UTC hour.
        // Bar-index arithmetic is unsound across weekend/missing-candle gaps,
        // so session/event gates need the real timestamp.
        let t = candle.time.timestamp();
        ctx.candle_time_unix = t;
        ctx.candle_hour = i64::from(chrono::Timelike::hour(&candle.time));

        // Event proximity (ABI v3): hours since the last calendar event at or
        // before this candle, and until the next one after it. -1 = unknown
        // (no calendar injected, or no event on that side).
        let (since, until) = event_proximity_hours(&self.event_times, t);
        ctx.hours_since_event = since;
        ctx.hours_until_event = until;

        // ABI v5 position state (pushed by the engine via sync_position_state;
        // hosts that never push leave the flat sentinels).
        match &self.position {
            Some(p) => {
                ctx.in_position = true;
                ctx.entry_price = p.entry_price;
                ctx.bars_since_entry = self.bars_in_position;
            }
            None => {
                ctx.in_position = false;
                ctx.entry_price = Decimal::ZERO;
                ctx.bars_since_entry = -1;
            }
        }

        // volume
        ctx.volume = candle.volume as i64;

        // bar_count
        ctx.bar_count = self.bar_count as i64;

        // params map
        let mut params_map = RhaiMap::new();
        for (k, v) in &self.resolved_params {
            let dec_val = Decimal::try_from(*v).unwrap_or_default();
            params_map.insert(k.clone().into(), Dynamic::from(dec_val));
        }
        ctx.params = params_map;

        // indicators: current values { indicator_id -> { output -> value } }
        let mut indicators_map = RhaiMap::new();
        for config in &self.indicator_configs {
            if let Some(history) = self.indicator_engine.get_history(&config.id) {
                let latest = history.get_all_latest();
                let mut output_map = RhaiMap::new();
                for (output_name, value_str) in &latest {
                    if let Ok(d) = value_str.parse::<Decimal>() {
                        output_map.insert(output_name.clone().into(), Dynamic::from(d));
                    }
                }
                indicators_map.insert(config.id.clone().into(), Dynamic::from(output_map));
            }
        }
        ctx.indicators = indicators_map;

        // indicator_history: { indicator_id -> { output -> [current, prev, ...] } }
        let mut history_map = RhaiMap::new();
        for config in &self.indicator_configs {
            let mut output_map = RhaiMap::new();
            if let Some(history) = self.indicator_engine.get_history(&config.id) {
                let latest = history.get_all_latest();
                for output_name in latest.keys() {
                    let mut arr = rhai::Array::new();
                    for offset in 0..INDICATOR_LOOKBACK {
                        match history.get(output_name, offset) {
                            Some(v) => arr.push(Dynamic::from(v)),
                            None => break,
                        }
                    }
                    output_map.insert(output_name.clone().into(), Dynamic::from(arr));
                }
            }
            history_map.insert(config.id.clone().into(), Dynamic::from(output_map));
        }
        ctx.indicator_history = history_map;

        // prices: array of candle maps (oldest first)
        let mut prices_arr = rhai::Array::new();
        for c in &self.price_history {
            let mut cm = RhaiMap::new();
            cm.insert("open".into(), Dynamic::from(c.mid.open));
            cm.insert("high".into(), Dynamic::from(c.mid.high));
            cm.insert("low".into(), Dynamic::from(c.mid.low));
            cm.insert("close".into(), Dynamic::from(c.mid.close));
            prices_arr.push(Dynamic::from(cm));
        }
        ctx.prices = prices_arr;
    }

    /// Parse the Dynamic result from `on_candle()` into an ExtendedSignal.
    fn parse_rhai_result(&mut self, result: Dynamic) -> ExtendedSignal {
        // If the result is a simple string, treat it as a signal name
        if let Some(s) = result.clone().try_cast::<rhai::ImmutableString>() {
            let signal = parse_signal_str(s.as_str());
            self.last_stop_loss = None;
            self.last_take_profit = None;
            self.last_exit_reason = None;
            self.last_entry_rule_name = None;
            return ExtendedSignal {
                signal,
                ..Default::default()
            };
        }

        // Otherwise expect a map
        let map = match result.try_cast::<RhaiMap>() {
            Some(m) => m,
            None => return ExtendedSignal::default(),
        };

        let signal = map
            .get("signal")
            .and_then(|v| v.clone().try_cast::<rhai::ImmutableString>())
            .map(|s| parse_signal_str(s.as_str()))
            .unwrap_or(Signal::Hold);

        let stop_loss = map
            .get("stop_loss")
            .and_then(|v| v.clone().try_cast::<Decimal>());

        let take_profit = map
            .get("take_profit")
            .and_then(|v| v.clone().try_cast::<Decimal>());

        let rule_name = map
            .get("rule_name")
            .and_then(|v| v.clone().try_cast::<rhai::ImmutableString>())
            .map(|s| s.to_string());

        let exit_reason = map
            .get("exit_reason")
            .and_then(|v| v.clone().try_cast::<rhai::ImmutableString>())
            .map(|s| s.to_string());

        let pending_order = map
            .get("pending_order")
            .and_then(|v| v.clone().try_cast::<RhaiMap>())
            .and_then(|po| parse_pending_order_map(&po));

        // Store for accessors
        self.last_stop_loss = stop_loss;
        self.last_take_profit = take_profit;
        self.last_exit_reason = exit_reason.clone();
        self.last_entry_rule_name = rule_name.clone();

        // Build indicator snapshot for entries
        let entry_indicators = match signal {
            Signal::Buy | Signal::Sell => {
                let snapshot = self.indicator_engine.get_snapshot();
                let mut flat: HashMap<String, String> = HashMap::new();
                for (indicator_id, outputs) in snapshot {
                    for (output_name, value) in outputs {
                        let key = if output_name == "value" {
                            indicator_id.clone()
                        } else {
                            format!("{}.{}", indicator_id, output_name)
                        };
                        flat.insert(key, value);
                    }
                }
                Some(flat)
            }
            _ => None,
        };

        ExtendedSignal {
            signal,
            stop_loss,
            take_profit,
            entry_rule_id: rule_name.clone(),
            entry_rule_name: rule_name,
            exit_reason,
            entry_indicators,
            pending_order,
        }
    }
}

impl ScriptedStrategy {
    /// Feed a warmup candle through the FULL evaluation path — indicators,
    /// price history, and the script's `on_candle()` — discarding the signal.
    ///
    /// Issue #9: this used to advance indicators/price history only,
    /// deliberately skipping the Rhai script "to avoid mutating user script
    /// state". That was exactly backwards for stateful scripts: every
    /// script-local state machine (setup references, breakout ranges, the
    /// script's own `position` var) started cold after a watcher (re)start,
    /// so any signal whose setup formed on warmup candles was missed live
    /// while `wickd strategy run` over identical candles fired. Replaying
    /// through [`Strategy::on_candle_extended`] keeps one source of truth
    /// for state evolution and matches backtest semantics exactly — and
    /// cannot double-advance indicators, since the full path is the only
    /// feed. Discarding the returned signal is the suppression: warmup
    /// candles are historical, never tradeable.
    pub fn warmup_candle(&mut self, candle: &Candle) {
        let _ = self.on_candle_extended(candle);
    }
}

impl Strategy for ScriptedStrategy {
    fn prepare(&mut self, _candles: &[Candle]) {
        // No-op: indicators are fed incrementally in on_candle_extended().
        // Do NOT feed candles here — the backtest engine calls on_candle_extended()
        // for every candle in sequence, so pre-feeding would double-process them
        // and corrupt indicator values + cross-detection logic.
    }

    fn on_candle(&mut self, candle: &Candle) -> Signal {
        self.on_candle_extended(candle).signal
    }

    fn on_candle_extended(&mut self, candle: &Candle) -> ExtendedSignal {
        // Check if we've had too many consecutive errors
        if self.consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
            return ExtendedSignal::default();
        }

        // 1. Feed candle to indicator engine
        self.indicator_engine.on_candle(candle);

        // 2. Add candle to price history
        self.price_history.push_back(candle.clone());
        if self.price_history.len() > self.max_price_history {
            self.price_history.pop_front();
        }

        // 3. Increment bar count
        self.bar_count += 1;

        // 3b. Pick up surprise-calendar CSV drops (throttled fingerprint
        // check) so a long-lived watcher sees new events and backfilled
        // actuals without a restart (AGT-632, AC1).
        if let Some(cal) = self.surprise.as_mut() {
            if cal.maybe_refresh() {
                if let Ok(mut ctx) = self.ctx.lock() {
                    ctx.surprise_releases = cal.releases();
                }
            }
        }

        // 4. Update shared context for SDK functions
        self.update_context(candle);

        // 5. Call Rhai on_candle(), bounded by the wall-clock guard (see
        // `register_wall_clock_guard`) in addition to the operation/array/map limits.
        reset_wall_clock(&self.wall_clock_start);
        let result: Result<Dynamic, _> =
            self.engine.call_fn(&mut self.scope, &self.ast, "on_candle", ());

        match result {
            Ok(dynamic) => {
                self.consecutive_errors = 0;
                self.abort_event_emitted = false;
                self.parse_rhai_result(dynamic)
            }
            Err(e) => {
                self.consecutive_errors += 1;
                if let Some(reason) = wall_clock_terminated_reason(&e) {
                    warn!(
                        strategy = %self.name,
                        reason = %reason,
                        consecutive = self.consecutive_errors,
                        "Rhai on_candle terminated by wall-clock guard, returning Hold"
                    );
                } else if self.consecutive_errors == MAX_CONSECUTIVE_ERRORS {
                    warn!(
                        strategy = %self.name,
                        "Rhai script aborted after {} consecutive errors. Last error: {}",
                        MAX_CONSECUTIVE_ERRORS,
                        e
                    );
                } else {
                    warn!(
                        strategy = %self.name,
                        error = %e,
                        consecutive = self.consecutive_errors,
                        "Rhai on_candle error, returning Hold"
                    );
                }
                ExtendedSignal::default()
            }
        }
    }

    fn current_stop_loss(&self) -> Option<Decimal> {
        self.last_stop_loss
    }

    fn current_take_profit(&self) -> Option<Decimal> {
        self.last_take_profit
    }

    fn sync_position_state(&mut self, position: Option<PositionSnapshot>) {
        self.bars_in_position = match (&self.position, &position) {
            // Same open position as last candle → one more bar elapsed.
            // (A position closed and reopened with an identical snapshot
            // between two syncs is indistinguishable and keeps counting —
            // acceptable: fills at different candles rarely price identically.)
            (Some(prev), Some(next)) if prev == next => self.bars_in_position + 1,
            _ => 0,
        };
        self.position = position;
    }

    fn notify_position_closed(&mut self) {
        // ABI v5: the position is gone — clear the snapshot and stage the
        // flat sentinels BEFORE the Rhai hook runs, so on_position_closed()
        // already sees in_position() == false.
        self.position = None;
        self.bars_in_position = 0;
        if let Ok(mut ctx) = self.ctx.lock() {
            ctx.in_position = false;
            ctx.entry_price = Decimal::ZERO;
            ctx.bars_since_entry = -1;
        }
        // Call optional on_position_closed() in Rhai
        reset_wall_clock(&self.wall_clock_start);
        let _ = self
            .engine
            .call_fn::<Dynamic>(&mut self.scope, &self.ast, "on_position_closed", ());
        self.last_stop_loss = None;
        self.last_take_profit = None;
        self.last_exit_reason = None;
        self.last_entry_rule_name = None;
    }

    fn notify_entry_rejected(&mut self) {
        self.last_stop_loss = None;
        self.last_take_profit = None;
        self.last_exit_reason = None;
        self.last_entry_rule_name = None;
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn reset(&mut self) {
        self.price_history.clear();
        self.indicator_engine.reset();
        self.bar_count = 0;
        self.consecutive_errors = 0;
        self.abort_event_emitted = false;
        self.last_stop_loss = None;
        self.last_take_profit = None;
        self.last_exit_reason = None;
        self.last_entry_rule_name = None;

        // ABI v5 position state does not survive a reset
        self.position = None;
        self.bars_in_position = 0;

        // Clear shared context (re-staging host-injected feeds, which — like
        // `event_times` — survive a reset)
        if let Ok(mut ctx) = self.ctx.lock() {
            *ctx = ScriptContext::default();
            ctx.bars_since_entry = -1;
            if let Some(cal) = &self.surprise {
                ctx.surprise_releases = cal.releases();
                ctx.surprise_legs = self.surprise_legs.clone();
            }
        }

        // Re-initialize scope and re-run top-level script code
        self.scope = Scope::new();
        reset_wall_clock(&self.wall_clock_start);
        if let Err(e) = self.engine.run_ast_with_scope(&mut self.scope, &self.ast) {
            warn!(strategy = %self.name, error = %e, "Failed to re-initialize script on reset");
        }
    }
}

// =============================================================================
// SDK function registration
// =============================================================================

/// Register all SDK functions on the engine. These are native Rust functions that
/// read from the shared `ScriptContext` via `Arc<Mutex<>>`.
/// Hours since the last event at-or-before `t` and until the first event
/// strictly after `t`, as exact Decimals; -1 for a side with no event (or an
/// empty calendar). `events` must be sorted ascending (see
/// [`ScriptedStrategy::set_event_calendar`]). Pure, binary-search based.
fn event_proximity_hours(events: &[i64], t: i64) -> (Decimal, Decimal) {
    const NONE: Decimal = Decimal::NEGATIVE_ONE;
    if events.is_empty() {
        return (NONE, NONE);
    }
    // partition_point: index of the first event strictly after `t`.
    let idx = events.partition_point(|e| *e <= t);
    let since = if idx == 0 {
        NONE
    } else {
        Decimal::from(t - events[idx - 1]) / Decimal::from(3600)
    };
    let until = if idx == events.len() {
        NONE
    } else {
        Decimal::from(events[idx] - t) / Decimal::from(3600)
    };
    (since, until)
}

/// Sentinel the `surprise_z()` family returns when no scored release matches
/// (no calendar injected, no release at or before the candle, filters match
/// nothing, or the latest release has no published actual yet). -9999 is far
/// outside any real z-score, so `z <= SURPRISE_Z_NONE` is unambiguous.
const SURPRISE_Z_NONE: Decimal = dec!(-9999);

/// Resolve the most recent scored surprise release for the current candle
/// under the given filters, returning `(z, hours_ago)` — both values always
/// refer to the SAME release, so scripts can gate a z on its recency.
/// `None` when nothing matches (including an unknown `min_impact` label,
/// consistent with the ABI's "typos degrade to inert" convention).
fn surprise_lookup(
    ctx: &ScriptContext,
    min_impact: &str,
    currency: Option<&str>,
) -> Option<(Decimal, Decimal)> {
    use rust_decimal::prelude::FromPrimitive;
    let min_rank = surprise::min_impact_rank(min_impact)?;
    let r = surprise::latest_scored(
        &ctx.surprise_releases,
        ctx.candle_time_unix,
        min_rank,
        &ctx.surprise_legs,
        currency,
    )?;
    let z = Decimal::from_f64(r.z)?;
    let hours = Decimal::from(ctx.candle_time_unix - r.time_unix) / Decimal::from(3600);
    Some((z, hours))
}

fn register_sdk_functions(engine: &mut Engine, ctx: Arc<Mutex<ScriptContext>>) {
    // price(field) -> Decimal
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("price", move |field: &str| -> Dynamic {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            match ctx.candle.get(field) {
                Some(v) => v.clone(),
                None => Dynamic::from(Decimal::ZERO),
            }
        });
    }

    // indicator(id, output) -> Decimal
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("indicator", move |id: &str, output: &str| -> Dynamic {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            ctx.indicators
                .get(id)
                .and_then(|v| v.clone().try_cast::<RhaiMap>())
                .and_then(|m| m.get(output).cloned())
                .unwrap_or_else(|| Dynamic::from(Decimal::ZERO))
        });
    }

    // indicator_at(id, output, offset) -> Decimal
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("indicator_at", move |id: &str, output: &str, offset: i64| -> Dynamic {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            ctx.indicator_history
                .get(id)
                .and_then(|v| v.clone().try_cast::<RhaiMap>())
                .and_then(|m| m.get(output).cloned())
                .and_then(|v| v.try_cast::<rhai::Array>())
                .and_then(|arr| {
                    let idx = offset as usize;
                    if idx < arr.len() { Some(arr[idx].clone()) } else { None }
                })
                .unwrap_or_else(|| Dynamic::from(Decimal::ZERO))
        });
    }

    // param(id) -> Decimal
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("param", move |id: &str| -> Dynamic {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            match ctx.params.get(id) {
                Some(v) => v.clone(),
                None => Dynamic::from(Decimal::ZERO),
            }
        });
    }

    // bar_count() -> i64
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("bar_count", move || -> i64 {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            ctx.bar_count
        });
    }

    // volume() -> i64
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("volume", move || -> i64 {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            ctx.volume
        });
    }

    // is_bullish() -> bool
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("is_bullish", move || -> bool {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            let close = ctx.candle.get("close").and_then(|v| v.clone().try_cast::<Decimal>()).unwrap_or_default();
            let open = ctx.candle.get("open").and_then(|v| v.clone().try_cast::<Decimal>()).unwrap_or_default();
            close > open
        });
    }

    // is_bearish() -> bool
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("is_bearish", move || -> bool {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            let close = ctx.candle.get("close").and_then(|v| v.clone().try_cast::<Decimal>()).unwrap_or_default();
            let open = ctx.candle.get("open").and_then(|v| v.clone().try_cast::<Decimal>()).unwrap_or_default();
            close < open
        });
    }

    // candle_range() -> Decimal
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("candle_range", move || -> Decimal {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            let high = ctx.candle.get("high").and_then(|v| v.clone().try_cast::<Decimal>()).unwrap_or_default();
            let low = ctx.candle.get("low").and_then(|v| v.clone().try_cast::<Decimal>()).unwrap_or_default();
            high - low
        });
    }

    // body_size() -> Decimal
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("body_size", move || -> Decimal {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            let close = ctx.candle.get("close").and_then(|v| v.clone().try_cast::<Decimal>()).unwrap_or_default();
            let open = ctx.candle.get("open").and_then(|v| v.clone().try_cast::<Decimal>()).unwrap_or_default();
            (close - open).abs()
        });
    }

    // price_at(field, offset) -> Decimal
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("price_at", move |field: &str, offset: i64| -> Dynamic {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            let len = ctx.prices.len();
            if len == 0 {
                return Dynamic::from(Decimal::ZERO);
            }
            let idx_from_end = offset as usize;
            if idx_from_end >= len {
                return Dynamic::from(Decimal::ZERO);
            }
            let idx = len - 1 - idx_from_end;
            if let Some(candle_dyn) = ctx.prices.get(idx) {
                if let Some(candle_map) = candle_dyn.clone().try_cast::<RhaiMap>() {
                    if let Some(val) = candle_map.get(field) {
                        return val.clone();
                    }
                }
            }
            Dynamic::from(Decimal::ZERO)
        });
    }

    // pip_value() -> Decimal
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("pip_value", move || -> Decimal {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            ctx.pip_value
        });
    }

    // candle_time() -> i64 — the current candle's open time as Unix seconds
    // (UTC). The script's clock: sound across weekend/missing-candle gaps
    // where bar-index arithmetic is not (ABI v2).
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("candle_time", move || -> i64 {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            ctx.candle_time_unix
        });
    }

    // candle_hour() -> i64 — the current candle's open hour, 0–23 UTC.
    // Convenience for session gates (ABI v2).
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("candle_hour", move || -> i64 {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            ctx.candle_hour
        });
    }

    // hours_since_event() -> Decimal — hours since the most recent calendar
    // event at or before this candle's open; -1 when unknown (ABI v3).
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("hours_since_event", move || -> Decimal {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            ctx.hours_since_event
        });
    }

    // hours_until_event() -> Decimal — hours until the next calendar event
    // after this candle's open; -1 when unknown (ABI v3).
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("hours_until_event", move || -> Decimal {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            ctx.hours_until_event
        });
    }

    // in_position() -> bool — whether the backtest engine holds an open
    // position for this strategy (ABI v5). Always false on hosts that do
    // not simulate positions (live watcher, `wickd strategy run`).
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("in_position", move || -> bool {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            ctx.in_position
        });
    }

    // entry_price() -> Decimal — the actual fill price of the open position
    // (ABI v5). 0 when flat: gate with in_position() first.
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("entry_price", move || -> Decimal {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            ctx.entry_price
        });
    }

    // bars_since_entry() -> i64 — completed candles since the position
    // opened (0 on the entry candle); -1 when flat (ABI v5).
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("bars_since_entry", move || -> i64 {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            ctx.bars_since_entry
        });
    }

    // surprise_z() family (ABI v4) — actual-vs-forecast z-score of the most
    // recent scored release at or before this candle's open, from the
    // updatable CSV calendar. Default filters: high impact, the instrument's
    // currency legs. -9999 = no matching scored release.
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("surprise_z", move || -> Decimal {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            surprise_lookup(&ctx, "high", None).map_or(SURPRISE_Z_NONE, |(z, _)| z)
        });
    }
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("surprise_z", move |min_impact: &str| -> Decimal {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            surprise_lookup(&ctx, min_impact, None).map_or(SURPRISE_Z_NONE, |(z, _)| z)
        });
    }
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("surprise_z_for", move |currency: &str| -> Decimal {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            surprise_lookup(&ctx, "high", Some(currency)).map_or(SURPRISE_Z_NONE, |(z, _)| z)
        });
    }
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn(
            "surprise_z_for",
            move |currency: &str, min_impact: &str| -> Decimal {
                let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
                surprise_lookup(&ctx, min_impact, Some(currency)).map_or(SURPRISE_Z_NONE, |(z, _)| z)
            },
        );
    }

    // surprise_hours_ago() family (ABI v4) — hours since the release the
    // matching surprise_z() call refers to (same filters ⇒ same release);
    // -1 = no matching scored release.
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("surprise_hours_ago", move || -> Decimal {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            surprise_lookup(&ctx, "high", None).map_or(Decimal::NEGATIVE_ONE, |(_, h)| h)
        });
    }
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("surprise_hours_ago", move |min_impact: &str| -> Decimal {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            surprise_lookup(&ctx, min_impact, None).map_or(Decimal::NEGATIVE_ONE, |(_, h)| h)
        });
    }
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("surprise_hours_ago_for", move |currency: &str| -> Decimal {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            surprise_lookup(&ctx, "high", Some(currency)).map_or(Decimal::NEGATIVE_ONE, |(_, h)| h)
        });
    }
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn(
            "surprise_hours_ago_for",
            move |currency: &str, min_impact: &str| -> Decimal {
                let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
                surprise_lookup(&ctx, min_impact, Some(currency))
                    .map_or(Decimal::NEGATIVE_ONE, |(_, h)| h)
            },
        );
    }

    // crossed_above(id1, out1, id2, out2) -> bool
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("crossed_above", move |id1: &str, out1: &str, id2: &str, out2: &str| -> bool {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            cross_check(&ctx.indicator_history, id1, out1, id2, out2, CrossDirection::Above)
        });
    }

    // crossed_below(id1, out1, id2, out2) -> bool
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("crossed_below", move |id1: &str, out1: &str, id2: &str, out2: &str| -> bool {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            cross_check(&ctx.indicator_history, id1, out1, id2, out2, CrossDirection::Below)
        });
    }

    // crossed_above_value(id, output, value) -> bool
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("crossed_above_value", move |id: &str, output: &str, value: Decimal| -> bool {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            cross_value_check(&ctx.indicator_history, id, output, value, CrossDirection::Above)
        });
    }

    // crossed_below_value(id, output, value) -> bool
    {
        let ctx = Arc::clone(&ctx);
        engine.register_fn("crossed_below_value", move |id: &str, output: &str, value: Decimal| -> bool {
            let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            cross_value_check(&ctx.indicator_history, id, output, value, CrossDirection::Below)
        });
    }

    // =========================================================================
    // Mixed Decimal/float arithmetic
    // =========================================================================
    // Rhai scripts use float literals (0.5, 2.0) but SDK functions return Decimal.
    // Without these, expressions like `atr * 0.5` cause silent type errors.

    use rust_decimal::prelude::FromPrimitive;

    // Decimal * f64 and f64 * Decimal
    engine.register_fn("*", |a: Decimal, b: f64| -> Decimal {
        a * Decimal::from_f64(b).unwrap_or(Decimal::ZERO)
    });
    engine.register_fn("*", |a: f64, b: Decimal| -> Decimal {
        Decimal::from_f64(a).unwrap_or(Decimal::ZERO) * b
    });

    // Decimal + f64 and f64 + Decimal
    engine.register_fn("+", |a: Decimal, b: f64| -> Decimal {
        a + Decimal::from_f64(b).unwrap_or(Decimal::ZERO)
    });
    engine.register_fn("+", |a: f64, b: Decimal| -> Decimal {
        Decimal::from_f64(a).unwrap_or(Decimal::ZERO) + b
    });

    // Decimal - f64 and f64 - Decimal
    engine.register_fn("-", |a: Decimal, b: f64| -> Decimal {
        a - Decimal::from_f64(b).unwrap_or(Decimal::ZERO)
    });
    engine.register_fn("-", |a: f64, b: Decimal| -> Decimal {
        Decimal::from_f64(a).unwrap_or(Decimal::ZERO) - b
    });

    // Decimal / f64 and f64 / Decimal
    // NaN/Inf/zero divisors return ZERO (safe fallback in backtest context)
    engine.register_fn("/", |a: Decimal, b: f64| -> Decimal {
        if !b.is_finite() || b == 0.0 { return Decimal::ZERO; }
        a / Decimal::from_f64(b).unwrap_or(Decimal::ONE)
    });
    engine.register_fn("/", |a: f64, b: Decimal| -> Decimal {
        if b.is_zero() { return Decimal::ZERO; }
        let a = if a.is_finite() { Decimal::from_f64(a).unwrap_or(Decimal::ZERO) } else { Decimal::ZERO };
        a / b
    });

    // Decimal comparisons with f64
    engine.register_fn(">", |a: Decimal, b: f64| -> bool {
        a > Decimal::from_f64(b).unwrap_or(Decimal::ZERO)
    });
    engine.register_fn("<", |a: Decimal, b: f64| -> bool {
        a < Decimal::from_f64(b).unwrap_or(Decimal::ZERO)
    });
    engine.register_fn(">=", |a: Decimal, b: f64| -> bool {
        a >= Decimal::from_f64(b).unwrap_or(Decimal::ZERO)
    });
    engine.register_fn("<=", |a: Decimal, b: f64| -> bool {
        a <= Decimal::from_f64(b).unwrap_or(Decimal::ZERO)
    });
    engine.register_fn("==", |a: Decimal, b: f64| -> bool {
        a == Decimal::from_f64(b).unwrap_or(Decimal::ZERO)
    });
    engine.register_fn("!=", |a: Decimal, b: f64| -> bool {
        a != Decimal::from_f64(b).unwrap_or(Decimal::ZERO)
    });

    // f64 comparisons with Decimal
    engine.register_fn(">", |a: f64, b: Decimal| -> bool {
        Decimal::from_f64(a).unwrap_or(Decimal::ZERO) > b
    });
    engine.register_fn("<", |a: f64, b: Decimal| -> bool {
        Decimal::from_f64(a).unwrap_or(Decimal::ZERO) < b
    });
    engine.register_fn(">=", |a: f64, b: Decimal| -> bool {
        Decimal::from_f64(a).unwrap_or(Decimal::ZERO) >= b
    });
    engine.register_fn("<=", |a: f64, b: Decimal| -> bool {
        Decimal::from_f64(a).unwrap_or(Decimal::ZERO) <= b
    });
    engine.register_fn("==", |a: f64, b: Decimal| -> bool {
        Decimal::from_f64(a).unwrap_or(Decimal::ZERO) == b
    });
    engine.register_fn("!=", |a: f64, b: Decimal| -> bool {
        Decimal::from_f64(a).unwrap_or(Decimal::ZERO) != b
    });
}

enum CrossDirection {
    Above,
    Below,
}

/// Check if indicator1.output1 crossed above/below indicator2.output2
fn cross_check(
    history: &RhaiMap,
    id1: &str, out1: &str,
    id2: &str, out2: &str,
    direction: CrossDirection,
) -> bool {
    let get_values = |id: &str, out: &str| -> Option<(Decimal, Decimal)> {
        let ind = history.get(id)?.clone().try_cast::<RhaiMap>()?;
        let arr = ind.get(out)?.clone().try_cast::<rhai::Array>()?;
        if arr.len() < 2 { return None; }
        let curr = arr[0].clone().try_cast::<Decimal>()?;
        let prev = arr[1].clone().try_cast::<Decimal>()?;
        Some((curr, prev))
    };

    let (curr1, prev1) = match get_values(id1, out1) {
        Some(v) => v,
        None => return false,
    };
    let (curr2, prev2) = match get_values(id2, out2) {
        Some(v) => v,
        None => return false,
    };

    match direction {
        CrossDirection::Above => prev1 <= prev2 && curr1 > curr2,
        CrossDirection::Below => prev1 >= prev2 && curr1 < curr2,
    }
}

/// Check if indicator crossed above/below a fixed value
fn cross_value_check(
    history: &RhaiMap,
    id: &str, output: &str,
    value: Decimal,
    direction: CrossDirection,
) -> bool {
    let ind = match history.get(id).and_then(|v| v.clone().try_cast::<RhaiMap>()) {
        Some(m) => m,
        None => return false,
    };
    let arr = match ind.get(output).and_then(|v| v.clone().try_cast::<rhai::Array>()) {
        Some(a) => a,
        None => return false,
    };
    if arr.len() < 2 { return false; }
    let curr = match arr[0].clone().try_cast::<Decimal>() {
        Some(v) => v,
        None => return false,
    };
    let prev = match arr[1].clone().try_cast::<Decimal>() {
        Some(v) => v,
        None => return false,
    };

    match direction {
        CrossDirection::Above => prev <= value && curr > value,
        CrossDirection::Below => prev >= value && curr < value,
    }
}

// =============================================================================
// Validation
// =============================================================================

/// Validate a Rhai script without creating a full strategy. Returns parsed metadata on success.
/// Typed failure from [`validate_script_typed`], so a caller can branch on the
/// *kind* of failure instead of substring-matching a human message. Its
/// `Display` form is byte-for-byte what the string-returning [`validate_script`]
/// wrapper emits, so every existing caller (the CLI load paths, the golden-corpus
/// test, and the desktop AI tool, which serializes the message verbatim) is
/// unaffected by the introduction of this type.
#[derive(Debug, Clone)]
pub enum ScriptValidationError {
    /// `@indicators` / `@parameters` metadata comment failed to parse.
    Metadata(String),
    /// The script failed to compile under the Rhai engine.
    Compile(String),
    /// The script compiled but defines no `on_candle()` function.
    MissingOnCandle,
}

impl std::fmt::Display for ScriptValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScriptValidationError::Metadata(m) => write!(f, "{m}"),
            ScriptValidationError::Compile(m) => write!(f, "Script compilation error: {m}"),
            ScriptValidationError::MissingOnCandle => {
                write!(f, "Script must define an `on_candle()` function with no parameters")
            }
        }
    }
}

impl std::error::Error for ScriptValidationError {}

/// Validate a script and return a **typed** error on failure. Runs the same
/// checks as [`validate_script`] — metadata parse, Rhai compile, `on_candle()`
/// presence — but the returned [`ScriptValidationError`] variant lets a caller
/// (e.g. `wickd strategy validate`) map the failure to a stable machine code
/// without inspecting the human-readable message text.
pub fn validate_script_typed(script: &str) -> Result<ScriptMetadata, ScriptValidationError> {
    let metadata = parse_metadata(script).map_err(ScriptValidationError::Metadata)?;

    let mut engine = Engine::new();
    configure_engine_limits(&mut engine);

    // Register SDK functions with a dummy context so compilation succeeds
    let dummy_ctx = Arc::new(Mutex::new(ScriptContext::default()));
    register_sdk_functions(&mut engine, dummy_ctx);

    let ast = engine
        .compile(script)
        .map_err(|e| ScriptValidationError::Compile(e.to_string()))?;

    // `verify_on_candle_exists` only ever fails one way (no `on_candle()`), so
    // its message is subsumed by the `MissingOnCandle` variant's `Display`.
    verify_on_candle_exists(&ast).map_err(|_| ScriptValidationError::MissingOnCandle)?;

    Ok(metadata)
}

/// String-returning validation used across the codebase (the CLI run/backtest
/// load paths, the golden-corpus test, the desktop AI tool). A thin wrapper over
/// [`validate_script_typed`] that flattens the typed error to its `Display`
/// string, preserving the exact message every existing caller already depends on.
pub fn validate_script(script: &str) -> Result<ScriptMetadata, String> {
    validate_script_typed(script).map_err(|e| e.to_string())
}

// =============================================================================
// Internal helpers
// =============================================================================

/// Check that the compiled AST contains an `on_candle` function.
fn verify_on_candle_exists(ast: &AST) -> Result<(), String> {
    let has_on_candle = ast
        .iter_functions()
        .any(|f| f.name == "on_candle" && f.params.is_empty());

    if !has_on_candle {
        return Err("Script must define an `on_candle()` function with no parameters".to_string());
    }

    Ok(())
}

fn parse_signal_str(s: &str) -> Signal {
    match s.to_lowercase().as_str() {
        "buy" | "long" => Signal::Buy,
        "sell" | "short" => Signal::Sell,
        "close" | "close_position" => Signal::ClosePosition,
        _ => Signal::Hold,
    }
}

/// Parse an `order_type` string from a script's `pending_order` map into an `EntryOrderType`.
/// Returns `None` for unrecognized strings (caller drops the pending order in that case).
fn parse_entry_order_type_str(s: &str) -> Option<EntryOrderType> {
    match s.to_lowercase().as_str() {
        "buy_stop" | "buystop" => Some(EntryOrderType::BuyStop),
        "sell_stop" | "sellstop" => Some(EntryOrderType::SellStop),
        "buy_limit" | "buylimit" => Some(EntryOrderType::BuyLimit),
        "sell_limit" | "selllimit" => Some(EntryOrderType::SellLimit),
        "market" => Some(EntryOrderType::Market),
        _ => None,
    }
}

/// Parse a script-returned `pending_order` map (`#{ order_type, price, expiry_bars }`) into a
/// `PendingOrderInfo`. Requires `order_type` and `price`; `expiry_bars` is optional.
/// Returns `None` if the map is missing required fields or `order_type` is unrecognized —
/// the entry then falls back to the standard market-order signal path.
fn parse_pending_order_map(map: &RhaiMap) -> Option<PendingOrderInfo> {
    let order_type = map
        .get("order_type")
        .and_then(|v| v.clone().try_cast::<rhai::ImmutableString>())
        .and_then(|s| parse_entry_order_type_str(s.as_str()))?;

    let price = map.get("price").and_then(|v| v.clone().try_cast::<Decimal>())?;

    let expiry_bars = map
        .get("expiry_bars")
        .and_then(|v| v.clone().try_cast::<i64>())
        .and_then(|n| u32::try_from(n).ok());

    Some(PendingOrderInfo {
        order_type,
        price,
        expiry_bars,
    })
}

fn pip_value_for_instrument(instrument: &str) -> Decimal {
    let upper = instrument.to_uppercase();
    if upper.contains("JPY") {
        dec!(0.01)
    } else if upper.contains("XAU") {
        dec!(0.1)
    } else if upper.contains("XAG") {
        dec!(0.001)
    } else {
        dec!(0.0001)
    }
}

// =============================================================================
// Metadata parsing
// =============================================================================

/// Parse `@indicators` and `@parameters` from script comments.
///
/// Format:
/// ```text
/// // @indicators: [ { "id": "ema_fast", "type": "ema", "params": { "period": 20 } } ]
/// // @parameters: [ { "id": "fast_period", "name": "Fast EMA", "type": "integer", "default": 9 } ]
/// ```
///
/// JSON may span multiple `//` comment lines.
fn parse_metadata(script: &str) -> Result<ScriptMetadata, String> {
    let indicators_json = extract_metadata_json(script, "@indicators:");
    let parameters_json = extract_metadata_json(script, "@parameters:");

    let indicators = match indicators_json {
        Some(json) => parse_indicators_json(&json)?,
        None => Vec::new(),
    };

    let parameters = match parameters_json {
        Some(json) => parse_parameters_json(&json)?,
        None => Vec::new(),
    };

    Ok(ScriptMetadata {
        indicators,
        parameters,
    })
}

/// Extract a JSON value that follows a `// @tag:` comment marker.
/// The JSON may span multiple `//` comment lines.
fn extract_metadata_json(script: &str, tag: &str) -> Option<String> {
    let lines: Vec<&str> = script.lines().collect();
    let mut json_str = String::new();
    let mut collecting = false;

    for line in &lines {
        let trimmed = line.trim();

        if !collecting {
            // Look for the tag in a comment line
            if let Some(stripped) = trimmed.strip_prefix("//") {
                let content = stripped.trim();
                if let Some(rest) = content.strip_prefix(tag) {
                    json_str.push_str(rest.trim());
                    collecting = true;

                    // Check if JSON is complete on this single line
                    if is_json_balanced(&json_str) {
                        return Some(json_str);
                    }
                }
            }
        } else {
            // Continue collecting from subsequent comment lines
            if let Some(stripped) = trimmed.strip_prefix("//") {
                json_str.push_str(stripped.trim());

                if is_json_balanced(&json_str) {
                    return Some(json_str);
                }
            } else {
                // Non-comment line while collecting: stop
                break;
            }
        }
    }

    if collecting && !json_str.is_empty() {
        Some(json_str)
    } else {
        None
    }
}

/// Rough check: string looks like JSON (starts with `[` or `{`) and brackets/braces are balanced.
fn is_json_balanced(s: &str) -> bool {
    let trimmed = s.trim();
    // Must start with a JSON container opener
    if !trimmed.starts_with('[') && !trimmed.starts_with('{') {
        return false;
    }
    let mut bracket_depth = 0i32;
    let mut brace_depth = 0i32;
    let mut in_string = false;
    let mut escape_next = false;

    for ch in s.chars() {
        if escape_next {
            escape_next = false;
            continue;
        }
        if ch == '\\' && in_string {
            escape_next = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        match ch {
            '[' => bracket_depth += 1,
            ']' => {
                bracket_depth -= 1;
                if bracket_depth < 0 { return false; }
            }
            '{' => brace_depth += 1,
            '}' => {
                brace_depth -= 1;
                if brace_depth < 0 { return false; }
            }
            _ => {}
        }
    }

    bracket_depth == 0 && brace_depth == 0
}

/// Parse the simplified indicator JSON into `IndicatorConfig` values.
fn parse_indicators_json(json: &str) -> Result<Vec<IndicatorConfig>, String> {
    let raw: Vec<serde_json::Value> = serde_json::from_str(json)
        .map_err(|e| format!("Failed to parse @indicators JSON: {}", e))?;

    let mut configs = Vec::new();
    for item in &raw {
        let id = item
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Indicator missing 'id' field".to_string())?
            .to_string();

        let type_str = item
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("Indicator '{}' missing 'type' field", id))?;

        let indicator_type = parse_indicator_type(type_str)
            .ok_or_else(|| format!("Unknown indicator type '{}' for '{}'", type_str, id))?;

        let params = match item.get("params") {
            Some(serde_json::Value::Object(obj)) => {
                let mut map = HashMap::new();
                for (k, v) in obj {
                    let pv = match v {
                        serde_json::Value::Number(n) => {
                            ParameterizedValue::Fixed(n.as_f64().unwrap_or(0.0))
                        }
                        serde_json::Value::Object(ref inner) => {
                            if let Some(param_id) = inner.get("$param").and_then(|p| p.as_str()) {
                                ParameterizedValue::Reference(shared::ParameterReference {
                                    param_id: param_id.to_string(),
                                })
                            } else {
                                return Err(format!(
                                    "Indicator '{}' param '{}': expected number or {{\"$param\": \"...\"}}", id, k
                                ));
                            }
                        }
                        _ => {
                            return Err(format!(
                                "Indicator '{}' param '{}': expected number or param reference", id, k
                            ));
                        }
                    };
                    map.insert(k.clone(), pv);
                }
                map
            }
            Some(_) => {
                return Err(format!("Indicator '{}': 'params' must be an object", id));
            }
            None => HashMap::new(),
        };

        configs.push(IndicatorConfig {
            id,
            indicator_type,
            params,
            symbol: None,
            timeframe: None,
        });
    }

    Ok(configs)
}

/// Parse a snake_case string to `IndicatorType`.
fn parse_indicator_type(s: &str) -> Option<IndicatorType> {
    // Use serde deserialization since IndicatorType is rename_all = "snake_case"
    let json = format!("\"{}\"", s);
    serde_json::from_str::<IndicatorType>(&json).ok()
}

/// Parse the simplified parameter JSON into `ParameterDefinition` values.
fn parse_parameters_json(json: &str) -> Result<Vec<ParameterDefinition>, String> {
    let raw: Vec<serde_json::Value> = serde_json::from_str(json)
        .map_err(|e| format!("Failed to parse @parameters JSON: {}", e))?;

    let mut params = Vec::new();
    for item in &raw {
        let id = item
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Parameter missing 'id' field".to_string())?
            .to_string();

        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&id)
            .to_string();

        let description = item
            .get("description")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let type_str = item
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("number");

        let param_type = match type_str {
            "integer" => ParameterType::Integer,
            "number" | "float" => ParameterType::Number,
            "select" => ParameterType::Select,
            "boolean" | "bool" => ParameterType::Boolean,
            _ => ParameterType::Number,
        };

        let default = item
            .get("default")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let min = item.get("min").and_then(|v| v.as_f64());
        let max = item.get("max").and_then(|v| v.as_f64());
        let step = item.get("step").and_then(|v| v.as_f64());

        let group = item
            .get("group")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let options = item.get("options").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|opt| {
                        let value = opt.get("value")?.as_f64()?;
                        let label = opt
                            .get("label")
                            .and_then(|l| l.as_str())
                            .unwrap_or("")
                            .to_string();
                        Some(ParameterOption { value, label })
                    })
                    .collect()
            })
        });

        params.push(ParameterDefinition {
            id,
            name,
            description,
            param_type,
            default,
            min,
            max,
            step,
            options,
            group,
        });
    }

    Ok(params)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Ohlc;
    use chrono::{DateTime, Duration, Utc};
    use rust_decimal_macros::dec;

    fn create_test_candle(price: Decimal, time_offset: i64) -> Candle {
        let base_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        Candle {
            time: base_time + Duration::hours(time_offset),
            mid: Ohlc {
                open: price - dec!(0.0010),
                high: price + dec!(0.0010),
                low: price - dec!(0.0020),
                close: price,
            },
            volume: 1000,
            complete: true,
        }
    }

    // -----------------------------------------------------------------------
    // 1. test_basic_signal
    // -----------------------------------------------------------------------
    #[test]
    fn test_basic_signal() {
        let script = r#"
fn on_candle() {
    "buy"
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "test_buy").unwrap();
        let candle = create_test_candle(dec!(1.1000), 0);
        let signal = strategy.on_candle(&candle);
        assert_eq!(signal, Signal::Buy);
    }

    // -----------------------------------------------------------------------
    // Issue #9: warmup must replay the script's state machine
    // -----------------------------------------------------------------------
    // A minimal stateful script in the rahagod shape: top-level `let` state
    // mutated by on_candle(). It arms on the first candle and only signals on
    // a later one — the signal exists ONLY if the arming candle ran through
    // the script.
    const ARMING_SCRIPT: &str = r#"
let armed = false;
fn on_candle() {
    if armed { "buy" } else { armed = true; "hold" }
}
"#;

    #[test]
    fn script_state_persists_across_candles() {
        // Sanity baseline for the warmup test below: the live path
        // accumulates script state across calls.
        let mut live = ScriptedStrategy::from_script(ARMING_SCRIPT, "state_live").unwrap();
        assert_eq!(live.on_candle(&create_test_candle(dec!(1.1000), 0)), Signal::Hold);
        assert_eq!(live.on_candle(&create_test_candle(dec!(1.1000), 1)), Signal::Buy);
    }

    #[test]
    fn warmup_replays_script_state() {
        // Issue #9 regression: the arming candle lands in warmup (a watcher
        // restart), and the signal candle is evaluated live. Before the fix
        // warmup skipped on_candle(), the script stayed cold, and this
        // returned Hold while `wickd strategy run` over the same candles
        // said Buy.
        let mut warmed = ScriptedStrategy::from_script(ARMING_SCRIPT, "state_warm").unwrap();
        warmed.warmup_candle(&create_test_candle(dec!(1.1000), 0));
        assert_eq!(warmed.on_candle(&create_test_candle(dec!(1.1000), 1)), Signal::Buy);
    }

    #[test]
    fn warmup_discards_signals_but_advances_everything_once() {
        // Warmup on a signal-producing candle must not surface the signal
        // anywhere (the discard IS the suppression) — but bar_count, price
        // history, and indicators must advance exactly once per candle,
        // identical to the live path over the same candles.
        let script = r#"
fn on_candle() {
    "buy"
}
"#;
        let mut warmed = ScriptedStrategy::from_script(script, "warm").unwrap();
        let mut live = ScriptedStrategy::from_script(script, "live").unwrap();

        for i in 0..3 {
            let c = create_test_candle(dec!(1.1000) + Decimal::from(i), i as i64);
            warmed.warmup_candle(&c);
            live.on_candle(&c);
        }

        assert_eq!(warmed.bar_count, 3);
        assert_eq!(warmed.bar_count, live.bar_count);
        assert_eq!(warmed.price_history.len(), live.price_history.len());
        assert_eq!(
            warmed.indicator_engine.get_snapshot(),
            live.indicator_engine.get_snapshot()
        );
    }

    // -----------------------------------------------------------------------
    // ABI v2: candle_time() / candle_hour() — the script's clock
    // -----------------------------------------------------------------------
    // A session gate: buy only during the Asian session (00:00–07:59 UTC).
    // Base time is 2024-01-01T00:00:00Z, so offset N hours → hour N % 24.
    #[test]
    fn test_candle_hour_session_gate() {
        let script = r#"
fn on_candle() {
    let h = candle_hour();
    if h >= 0 && h < 8 { "buy" } else { "hold" }
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "session_gate").unwrap();
        // 03:00 UTC — inside the gate.
        assert_eq!(strategy.on_candle(&create_test_candle(dec!(1.1000), 3)), Signal::Buy);
        // 14:00 UTC — outside.
        assert_eq!(strategy.on_candle(&create_test_candle(dec!(1.1000), 14)), Signal::Hold);
        // 27h offset = 03:00 UTC next day — inside again (hour, not bar index).
        assert_eq!(strategy.on_candle(&create_test_candle(dec!(1.1000), 27)), Signal::Buy);
    }

    // -----------------------------------------------------------------------
    // ABI v3: hours_since_event() / hours_until_event()
    // -----------------------------------------------------------------------
    // The H-012 pattern: a reversion blackout for 72h after a calendar event.
    // Base time 2024-01-01T00:00:00Z; one event at +10h.
    #[test]
    fn test_event_blackout_gate() {
        let script = r#"
fn on_candle() {
    let h = hours_since_event();
    if h >= 0.0 && h < 72.0 { "hold" } else { "buy" }
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "blackout").unwrap();
        let base = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        strategy.set_event_calendar(vec![base + Duration::hours(10)]);

        // Before the event: no prior event → -1 → not blacked out.
        assert_eq!(strategy.on_candle(&create_test_candle(dec!(1.1), 5)), Signal::Buy);
        // 2h after the event: inside the 72h blackout.
        assert_eq!(strategy.on_candle(&create_test_candle(dec!(1.1), 12)), Signal::Hold);
        // 71h after: still inside.
        assert_eq!(strategy.on_candle(&create_test_candle(dec!(1.1), 81)), Signal::Hold);
        // 72h after: blackout over.
        assert_eq!(strategy.on_candle(&create_test_candle(dec!(1.1), 82)), Signal::Buy);
    }

    // The pure proximity math: sides resolve independently, -1 when absent,
    // fractional hours are exact Decimals.
    #[test]
    fn test_event_proximity_hours() {
        let none = Decimal::NEGATIVE_ONE;
        assert_eq!(event_proximity_hours(&[], 100), (none, none));

        let events = [3600, 7200]; // 01:00 and 02:00 (unix seconds)
        // Before the first event: no `since`, 1h until.
        assert_eq!(event_proximity_hours(&events, 0), (none, Decimal::ONE));
        // Exactly on an event: since = 0, until = next.
        assert_eq!(event_proximity_hours(&events, 3600), (Decimal::ZERO, Decimal::ONE));
        // Between events, fractional: 30min after the first.
        let (since, until) = event_proximity_hours(&events, 5400);
        assert_eq!(since, dec!(0.5));
        assert_eq!(until, dec!(0.5));
        // After the last event: no `until`.
        assert_eq!(event_proximity_hours(&events, 10800), (Decimal::ONE, none));
    }

    // -----------------------------------------------------------------------
    // ABI v4: surprise_z() / surprise_hours_ago() — the live surprise feed
    // -----------------------------------------------------------------------

    /// A candle at an absolute UTC time (the v3 helper is offset-based, but
    /// surprise fixtures pin absolute release times).
    fn candle_at(rfc3339: &str) -> Candle {
        Candle {
            time: DateTime::parse_from_rfc3339(rfc3339).unwrap().with_timezone(&Utc),
            mid: Ohlc {
                open: dec!(1.0990),
                high: dec!(1.1010),
                low: dec!(1.0980),
                close: dec!(1.1000),
            },
            volume: 1000,
            complete: true,
        }
    }

    /// Calendar fixture: a USD "CPI y/y" series with 8 discovery releases of
    /// surprise ±1 (mean 0, pstdev 1 — so z == raw surprise), plus whatever
    /// extra rows the test appends. Forecast fixed at 3.0.
    fn surprise_fixture_csv(extra_rows: &str) -> String {
        let mut s = String::from(surprise::FF_CSV_HEADER);
        s.push('\n');
        for i in 0..8 {
            let actual = if i % 2 == 0 { 4.0 } else { 2.0 };
            s.push_str(&format!("2024-0{}-10,13:30,USD,CPI y/y,high,{actual}%,3.0%,3.0%\n", i + 1));
        }
        s.push_str(extra_rows);
        s
    }

    fn surprise_calendar_from(csv: &str) -> (tempfile::TempDir, SurpriseCalendar) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("cal.csv"), csv).unwrap();
        let mut cal = SurpriseCalendar::load_dir(dir.path()).unwrap();
        cal.set_refresh_interval(std::time::Duration::ZERO); // deterministic reloads in tests
        (dir, cal)
    }

    // The H-015-live shape: act only 0–24h after a |z| > 1.5 high-impact
    // release on the instrument's legs; both accessors refer to the same
    // release, so the recency gate is sound.
    const SURPRISE_FADE_SCRIPT: &str = r#"
fn on_candle() {
    let z = surprise_z();
    if z <= -9999.0 { return "hold"; }          // sentinel: no scored release
    if z < 1.5 && z > -1.5 { return "hold"; }   // only BIG surprises
    let hrs = surprise_hours_ago();
    if hrs < 0.0 || hrs > 24.0 { return "hold"; }
    "sell"
}
"#;

    #[test]
    fn test_surprise_z_gates_on_magnitude_and_recency() {
        // One post-discovery release with surprise +2 → z = +2.
        let csv = surprise_fixture_csv("2024-09-01,10:00,USD,CPI y/y,high,5.0%,3.0%,3.1%\n");
        let (_dir, cal) = surprise_calendar_from(&csv);
        let mut strategy = ScriptedStrategy::from_script(SURPRISE_FADE_SCRIPT, "h015").unwrap();
        strategy.set_surprise_calendar(cal, "EUR_USD");

        // Before the big release the latest scored release is 2024-08-10
        // (|z| = 1): magnitude gate holds. No lookahead to the 10:00 release.
        assert_eq!(strategy.on_candle(&candle_at("2024-09-01T05:00:00Z")), Signal::Hold);
        // 2h after the z = +2 release: fade.
        assert_eq!(strategy.on_candle(&candle_at("2024-09-01T12:00:00Z")), Signal::Sell);
        // 25h after: same release, but outside the recency window.
        assert_eq!(strategy.on_candle(&candle_at("2024-09-02T11:00:00Z")), Signal::Hold);
    }

    #[test]
    fn test_surprise_sentinels_without_a_calendar() {
        let script = r#"
fn on_candle() {
    if surprise_z() <= -9999.0 && surprise_hours_ago() < 0.0 { "buy" } else { "sell" }
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "sentinels").unwrap();
        // No calendar injected → both accessors sit at their sentinels.
        assert_eq!(strategy.on_candle(&candle_at("2024-09-01T12:00:00Z")), Signal::Buy);
    }

    #[test]
    fn test_surprise_currency_and_impact_siblings() {
        // A JPY series (not a EUR_USD leg) and a medium-impact EUR series,
        // both scored, both more recent than the last USD release.
        let mut extra = String::new();
        for i in 0..8 {
            let actual = if i % 2 == 0 { 4.0 } else { 2.0 };
            extra.push_str(&format!("2024-0{}-20,01:30,JPY,Tankan,high,{actual},3.0,3.0\n", i + 1));
            extra.push_str(&format!("2024-0{}-22,09:00,EUR,PMI,medium,{actual},3.0,3.0\n", i + 1));
        }
        extra.push_str("2024-09-02,01:30,JPY,Tankan,high,5.0,3.0,3.0\n"); // z = +2
        let csv = surprise_fixture_csv(&extra);
        let (_dir, cal) = surprise_calendar_from(&csv);

        let script = r#"
fn on_candle() {
    if surprise_z_for("JPY") > 1.5 { return "buy" }         // non-leg currency, explicit
    if surprise_z("medium") <= -9999.0 { return "sell" }    // impact threshold widens the match
    "hold"
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "siblings").unwrap();
        strategy.set_surprise_calendar(cal, "EUR_USD");
        // The JPY z = +2 release is visible via surprise_z_for despite JPY not
        // being a EUR_USD leg (the default-leg filter is what surprise_z uses).
        assert_eq!(strategy.on_candle(&candle_at("2024-09-02T03:00:00Z")), Signal::Buy);
    }

    #[test]
    fn test_running_strategy_sees_backfilled_actuals_without_reload() {
        // The AC1 flow: a release is published (forecast only), the running
        // strategy holds; the monthly CSV is re-dropped with the actual
        // backfilled; the SAME strategy instance now sees the surprise.
        let pending = surprise_fixture_csv("2024-09-01,10:00,USD,CPI y/y,high,,3.0%,3.1%\n");
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("cal.csv"), &pending).unwrap();
        let mut cal = SurpriseCalendar::load_dir(dir.path()).unwrap();
        cal.set_refresh_interval(std::time::Duration::ZERO);

        let mut strategy = ScriptedStrategy::from_script(SURPRISE_FADE_SCRIPT, "h015").unwrap();
        strategy.set_surprise_calendar(cal, "EUR_USD");

        // Actual not yet published → the pending release is skipped; the
        // latest scored release (2024-08-10, |z| = 1) fails the magnitude gate.
        assert_eq!(strategy.on_candle(&candle_at("2024-09-01T12:00:00Z")), Signal::Hold);

        // Backfill the actual (surprise +2 → z = +2) by re-dropping the CSV.
        let backfilled = surprise_fixture_csv("2024-09-01,10:00,USD,CPI y/y,high,5.0%,3.0%,3.1%\n");
        std::fs::write(dir.path().join("cal.csv"), &backfilled).unwrap();

        // Next candle on the SAME instance: the refresh hook picks it up.
        assert_eq!(strategy.on_candle(&candle_at("2024-09-01T13:00:00Z")), Signal::Sell);
    }

    // candle_time() is the candle's open as Unix seconds, exact and monotonic
    // across gaps — scripts can measure real elapsed time between candles.
    #[test]
    fn test_candle_time_unix_seconds() {
        // 2024-01-01T00:00:00Z = 1704067200; the script buys only when the
        // candle is at least 48 REAL hours past that epoch base.
        let script = r#"
fn on_candle() {
    if candle_time() >= 1704067200 + 48 * 3600 { "buy" } else { "hold" }
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "time_gate").unwrap();
        assert_eq!(strategy.on_candle(&create_test_candle(dec!(1.1000), 0)), Signal::Hold);
        assert_eq!(strategy.on_candle(&create_test_candle(dec!(1.1000), 47)), Signal::Hold);
        assert_eq!(strategy.on_candle(&create_test_candle(dec!(1.1000), 48)), Signal::Buy);
    }

    // -----------------------------------------------------------------------
    // 2. test_hold_signal
    // -----------------------------------------------------------------------
    #[test]
    fn test_hold_signal() {
        let script = r#"
fn on_candle() {
    "hold"
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "test_hold").unwrap();
        let candle = create_test_candle(dec!(1.1000), 0);
        let signal = strategy.on_candle(&candle);
        assert_eq!(signal, Signal::Hold);
    }

    // -----------------------------------------------------------------------
    // 3. test_sell_signal
    // -----------------------------------------------------------------------
    #[test]
    fn test_sell_signal() {
        let script = r#"
fn on_candle() {
    "sell"
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "test_sell").unwrap();
        let candle = create_test_candle(dec!(1.1000), 0);
        let signal = strategy.on_candle(&candle);
        assert_eq!(signal, Signal::Sell);
    }

    // -----------------------------------------------------------------------
    // 4. test_close_signal
    // -----------------------------------------------------------------------
    #[test]
    fn test_close_signal() {
        let script = r#"
fn on_candle() {
    #{ signal: "close", exit_reason: "Take profit hit" }
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "test_close").unwrap();
        let candle = create_test_candle(dec!(1.1000), 0);
        let ext = strategy.on_candle_extended(&candle);
        assert_eq!(ext.signal, Signal::ClosePosition);
        assert_eq!(ext.exit_reason.as_deref(), Some("Take profit hit"));
    }

    // -----------------------------------------------------------------------
    // 5. test_indicator_access
    // -----------------------------------------------------------------------
    #[test]
    fn test_indicator_access() {
        let script = r#"
// @indicators: [{ "id": "rsi", "type": "rsi", "params": { "period": 14 } }]

fn on_candle() {
    let rsi_val = indicator("rsi", "value");
    if rsi_val > 70.0 {
        "sell"
    } else if rsi_val < 30.0 {
        "buy"
    } else {
        "hold"
    }
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "test_rsi").unwrap();

        // Feed enough candles with rising prices to push RSI high
        for i in 0..30 {
            let price = dec!(1.1000) + Decimal::from(i) * dec!(0.0010);
            let candle = create_test_candle(price, i);
            strategy.on_candle(&candle);
        }

        // After 30 candles of steady rise, RSI should be high
        let rsi_val = strategy
            .indicator_engine
            .get_latest("rsi", "value")
            .unwrap();
        // Just verify indicator was computed and accessible
        assert!(rsi_val > dec!(0));
    }

    // -----------------------------------------------------------------------
    // 6. test_state_persistence
    // -----------------------------------------------------------------------
    #[test]
    fn test_state_persistence() {
        let script = r#"
let counter = 0;

fn on_candle() {
    counter += 1;
    if counter >= 3 {
        "buy"
    } else {
        "hold"
    }
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "test_state").unwrap();
        let candle = create_test_candle(dec!(1.1000), 0);

        assert_eq!(strategy.on_candle(&candle), Signal::Hold);
        assert_eq!(strategy.on_candle(&candle), Signal::Hold);
        assert_eq!(strategy.on_candle(&candle), Signal::Buy);
        assert_eq!(strategy.on_candle(&candle), Signal::Buy);
    }

    // -----------------------------------------------------------------------
    // 7. test_parameter_override
    // -----------------------------------------------------------------------
    #[test]
    fn test_parameter_override() {
        let script = r#"
// @parameters: [{ "id": "threshold", "name": "Threshold", "type": "number", "default": 50.0 }]

fn on_candle() {
    let t = param("threshold");
    if t > 90.0 {
        "buy"
    } else {
        "hold"
    }
}
"#;
        // With default (50.0) -> hold
        let mut strategy = ScriptedStrategy::from_script(script, "test_param").unwrap();
        let candle = create_test_candle(dec!(1.1000), 0);
        assert_eq!(strategy.on_candle(&candle), Signal::Hold);

        // With override (100.0) -> buy
        let mut overrides = HashMap::new();
        overrides.insert("threshold".to_string(), 100.0);
        let mut strategy2 =
            ScriptedStrategy::from_script_with_params(script, "test_param2", overrides).unwrap();
        assert_eq!(strategy2.on_candle(&candle), Signal::Buy);
    }

    // -----------------------------------------------------------------------
    // 8. test_error_recovery
    // -----------------------------------------------------------------------
    #[test]
    fn test_error_recovery() {
        let script = r#"
fn on_candle() {
    let x = 1 / 0;
    "buy"
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "test_error").unwrap();
        let candle = create_test_candle(dec!(1.1000), 0);
        // Should not panic, should return Hold
        let signal = strategy.on_candle(&candle);
        assert_eq!(signal, Signal::Hold);
    }

    // -----------------------------------------------------------------------
    // 9. test_crossed_above
    // -----------------------------------------------------------------------
    #[test]
    fn test_crossed_above() {
        let script = r#"
// @indicators: [
//   { "id": "ema_fast", "type": "ema", "params": { "period": 3 } },
//   { "id": "ema_slow", "type": "ema", "params": { "period": 10 } }
// ]

fn on_candle() {
    if crossed_above("ema_fast", "value", "ema_slow", "value") {
        "buy"
    } else {
        "hold"
    }
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "test_cross").unwrap();

        // Feed declining prices so fast EMA < slow EMA
        for i in 0..15 {
            let price = dec!(1.2000) - Decimal::from(i) * dec!(0.0010);
            let candle = create_test_candle(price, i);
            strategy.on_candle(&candle);
        }

        // Now feed sharply rising prices to trigger a crossover
        let mut found_buy = false;
        for i in 15..30 {
            let price = dec!(1.1850) + Decimal::from(i - 15) * dec!(0.0030);
            let candle = create_test_candle(price, i);
            let signal = strategy.on_candle(&candle);
            if signal == Signal::Buy {
                found_buy = true;
                break;
            }
        }
        assert!(found_buy, "Expected a Buy signal from crossed_above detection");
    }

    // -----------------------------------------------------------------------
    // 9b. test_decimal_float_arithmetic
    // -----------------------------------------------------------------------
    #[test]
    fn test_decimal_float_arithmetic() {
        let script = r#"
// @indicators: [{ "id": "atr", "type": "atr", "params": { "period": 14 } }]

fn on_candle() {
    let close = price("close");
    let atr = indicator("atr", "value");

    // This exercises Decimal * float, Decimal - Decimal, Decimal + Decimal*float
    let sl = close - atr * 0.5;
    let risk = close - sl;
    let tp = close + risk * 2.0;

    if tp > close {
        return #{ signal: "buy", stop_loss: sl, take_profit: tp };
    }
    #{ signal: "hold" }
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "test_arith").unwrap();

        // Feed enough candles to warm up ATR (14 periods)
        let mut found_buy = false;
        for i in 0..30 {
            let price = dec!(1.1000) + Decimal::from(i % 5) * dec!(0.0010);
            let candle = create_test_candle(price, i);
            let ext = strategy.on_candle_extended(&candle);
            if ext.signal == Signal::Buy {
                found_buy = true;
                assert!(ext.stop_loss.is_some(), "Expected stop_loss to be set");
                assert!(ext.take_profit.is_some(), "Expected take_profit to be set");
                break;
            }
        }
        assert!(found_buy, "Expected Buy from Decimal*float arithmetic - got only Hold (type error?)");
    }

    // -----------------------------------------------------------------------
    // 9c. test_precompile_from_precompiled
    // -----------------------------------------------------------------------
    #[test]
    fn test_precompile_from_precompiled() {
        let script = r#"
// @parameters: [
//   { "id": "threshold", "name": "Threshold", "type": "integer", "default": 5, "min": 3, "max": 10, "step": 1 }
// ]
// @indicators: [{ "id": "atr", "type": "atr", "params": { "period": 14 } }]

let counter = 0;

fn on_candle() {
    counter += 1;
    let thresh = param("threshold");
    if counter >= thresh {
        return #{ signal: "buy" };
    }
    #{ signal: "hold" }
}
"#;
        // Precompile once
        let (metadata, ast) = ScriptedStrategy::precompile(script).unwrap();
        assert_eq!(metadata.parameters.len(), 1);
        assert_eq!(metadata.parameters[0].id, "threshold");

        // Create two instances with different param overrides
        let mut params_a = HashMap::new();
        params_a.insert("threshold".to_string(), 3.0);
        let mut strategy_a = ScriptedStrategy::from_precompiled(&metadata, &ast, "test_a", params_a).unwrap();

        let mut params_b = HashMap::new();
        params_b.insert("threshold".to_string(), 7.0);
        let mut strategy_b = ScriptedStrategy::from_precompiled(&metadata, &ast, "test_b", params_b).unwrap();

        // Strategy A (threshold=3) should buy on candle 3
        let mut a_bought_at = None;
        let mut b_bought_at = None;
        for i in 0..10 {
            let c = create_test_candle(dec!(1.1000), i);
            if strategy_a.on_candle(&c) == Signal::Buy && a_bought_at.is_none() {
                a_bought_at = Some(i);
            }
            if strategy_b.on_candle(&c) == Signal::Buy && b_bought_at.is_none() {
                b_bought_at = Some(i);
            }
        }

        assert_eq!(a_bought_at, Some(2), "Strategy A (threshold=3) should buy on 3rd candle (idx 2)");
        assert_eq!(b_bought_at, Some(6), "Strategy B (threshold=7) should buy on 7th candle (idx 6)");
    }

    // -----------------------------------------------------------------------
    // 10. test_reset
    // -----------------------------------------------------------------------
    #[test]
    fn test_reset() {
        let script = r#"
let counter = 0;

fn on_candle() {
    counter += 1;
    if counter >= 3 {
        "buy"
    } else {
        "hold"
    }
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "test_reset").unwrap();
        let candle = create_test_candle(dec!(1.1000), 0);

        // Run to buy
        strategy.on_candle(&candle);
        strategy.on_candle(&candle);
        assert_eq!(strategy.on_candle(&candle), Signal::Buy);

        // Reset should clear state
        strategy.reset();
        assert_eq!(strategy.bar_count, 0);
        assert!(strategy.price_history.is_empty());

        // Counter should be back to 0
        assert_eq!(strategy.on_candle(&candle), Signal::Hold);
        assert_eq!(strategy.on_candle(&candle), Signal::Hold);
        assert_eq!(strategy.on_candle(&candle), Signal::Buy);
    }

    // -----------------------------------------------------------------------
    // 11. test_extended_signal
    // -----------------------------------------------------------------------
    #[test]
    fn test_extended_signal() {
        let script = r#"
fn on_candle() {
    #{
        signal: "buy",
        stop_loss: price("close") - 0.0050,
        take_profit: price("close") + 0.0100,
        rule_name: "My Rule"
    }
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "test_extended").unwrap();
        let candle = create_test_candle(dec!(1.1000), 0);
        let ext = strategy.on_candle_extended(&candle);

        assert_eq!(ext.signal, Signal::Buy);
        assert!(ext.stop_loss.is_some());
        assert!(ext.take_profit.is_some());
        assert_eq!(ext.entry_rule_name.as_deref(), Some("My Rule"));

        // stop_loss = close (1.1000) - 0.0050 = 1.0950
        assert_eq!(ext.stop_loss.unwrap(), dec!(1.1000) - dec!(0.0050));
        // take_profit = close (1.1000) + 0.0100 = 1.1100
        assert_eq!(ext.take_profit.unwrap(), dec!(1.1000) + dec!(0.0100));
    }

    // -----------------------------------------------------------------------
    // 11b. test_pending_order_parsed
    // -----------------------------------------------------------------------
    #[test]
    fn test_pending_order_parsed() {
        let script = r#"
fn on_candle() {
    #{
        signal: "buy",
        stop_loss: 1.0950,
        pending_order: #{ order_type: "buy_stop", price: 1.1050, expiry_bars: 5 }
    }
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "test_pending").unwrap();
        let candle = create_test_candle(dec!(1.1000), 0);
        let ext = strategy.on_candle_extended(&candle);

        assert_eq!(ext.signal, Signal::Buy);
        let pending = ext.pending_order.expect("Expected pending_order to be parsed");
        assert_eq!(pending.order_type, shared::EntryOrderType::BuyStop);
        assert_eq!(pending.price, dec!(1.1050));
        assert_eq!(pending.expiry_bars, Some(5));
    }

    // -----------------------------------------------------------------------
    // 11c. test_pending_order_all_order_types
    // -----------------------------------------------------------------------
    #[test]
    fn test_pending_order_all_order_types() {
        let cases = [
            ("buy_stop", shared::EntryOrderType::BuyStop),
            ("sell_stop", shared::EntryOrderType::SellStop),
            ("buy_limit", shared::EntryOrderType::BuyLimit),
            ("sell_limit", shared::EntryOrderType::SellLimit),
        ];

        for (order_type_str, expected) in cases {
            let script = format!(
                r#"
fn on_candle() {{
    #{{ signal: "buy", pending_order: #{{ order_type: "{}", price: 1.1000 }} }}
}}
"#,
                order_type_str
            );
            let mut strategy = ScriptedStrategy::from_script(&script, "test_pending_types").unwrap();
            let candle = create_test_candle(dec!(1.1000), 0);
            let ext = strategy.on_candle_extended(&candle);

            let pending = ext
                .pending_order
                .unwrap_or_else(|| panic!("Expected pending_order for order_type '{}'", order_type_str));
            assert_eq!(pending.order_type, expected, "Mismatched order_type for '{}'", order_type_str);
            // No expiry_bars given — should default to None
            assert_eq!(pending.expiry_bars, None);
        }
    }

    // -----------------------------------------------------------------------
    // 11d. test_pending_order_missing_fields_dropped
    // -----------------------------------------------------------------------
    #[test]
    fn test_pending_order_missing_fields_dropped() {
        // No `price` field and an unrecognized `order_type` — both should fail to
        // parse gracefully, leaving pending_order as None (falls back to market signal).
        let script = r#"
fn on_candle() {
    #{ signal: "buy", pending_order: #{ order_type: "not_a_real_type" } }
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "test_pending_bad").unwrap();
        let candle = create_test_candle(dec!(1.1000), 0);
        let ext = strategy.on_candle_extended(&candle);

        assert_eq!(ext.signal, Signal::Buy);
        assert!(ext.pending_order.is_none(), "Malformed pending_order should be dropped, not panic");
    }

    // -----------------------------------------------------------------------
    // 12. test_sandbox_max_operations
    // -----------------------------------------------------------------------
    #[test]
    fn test_sandbox_max_operations() {
        let script = r#"
fn on_candle() {
    let i = 0;
    loop {
        i += 1;
    }
    "buy"
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "test_sandbox").unwrap();
        let candle = create_test_candle(dec!(1.1000), 0);

        // Should not hang — engine terminates after max operations, returns Hold
        let signal = strategy.on_candle(&candle);
        assert_eq!(signal, Signal::Hold);
    }

    // -----------------------------------------------------------------------
    // 13. test_metadata_parsing
    // -----------------------------------------------------------------------
    #[test]
    fn test_metadata_parsing() {
        let script = r#"
// @indicators: [
//   { "id": "ema_fast", "type": "ema", "params": { "period": { "$param": "fast_period" } } },
//   { "id": "rsi", "type": "rsi", "params": { "period": 14 } }
// ]
// @parameters: [
//   { "id": "fast_period", "name": "Fast EMA Period", "type": "integer", "default": 9, "min": 5, "max": 50, "step": 1 }
// ]

fn on_candle() {
    "hold"
}
"#;
        let metadata = parse_metadata(script).unwrap();

        assert_eq!(metadata.indicators.len(), 2);
        assert_eq!(metadata.indicators[0].id, "ema_fast");
        assert_eq!(metadata.indicators[0].indicator_type, IndicatorType::Ema);
        assert_eq!(metadata.indicators[1].id, "rsi");
        assert_eq!(metadata.indicators[1].indicator_type, IndicatorType::Rsi);

        assert_eq!(metadata.parameters.len(), 1);
        assert_eq!(metadata.parameters[0].id, "fast_period");
        assert_eq!(metadata.parameters[0].default, 9.0);
        assert_eq!(metadata.parameters[0].min, Some(5.0));
        assert_eq!(metadata.parameters[0].max, Some(50.0));
        assert_eq!(metadata.parameters[0].step, Some(1.0));
        assert_eq!(metadata.parameters[0].param_type, ParameterType::Integer);
    }

    // -----------------------------------------------------------------------
    // 14. test_validate_script_missing_on_candle
    // -----------------------------------------------------------------------
    #[test]
    fn test_validate_script_missing_on_candle() {
        let script = r#"
fn not_on_candle() {
    "buy"
}
"#;
        let result = validate_script(script);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("on_candle"));
    }

    // -----------------------------------------------------------------------
    // AGT-609: typed validation error so callers branch on variants, not text
    // -----------------------------------------------------------------------
    #[test]
    fn test_validate_script_typed_classifies_each_failure_and_displays_like_the_string_wrapper() {
        // Compile failure → Compile variant, and its Display equals what the
        // string wrapper returns (so existing callers see no change).
        let bad = r#"fn on_candle( { "buy" }"#;
        let typed = validate_script_typed(bad).unwrap_err();
        assert!(matches!(typed, ScriptValidationError::Compile(_)));
        assert_eq!(typed.to_string(), validate_script(bad).unwrap_err());

        // Missing on_candle → MissingOnCandle variant, Display mentions on_candle.
        let missing = r#"fn not_on_candle() { "buy" }"#;
        let typed = validate_script_typed(missing).unwrap_err();
        assert!(matches!(typed, ScriptValidationError::MissingOnCandle));
        assert!(typed.to_string().contains("on_candle"));

        // Malformed @parameters JSON → Metadata variant.
        let bad_meta = "// @parameters: [ { not valid json\nfn on_candle() { \"hold\" }";
        let typed = validate_script_typed(bad_meta).unwrap_err();
        assert!(matches!(typed, ScriptValidationError::Metadata(_)));

        // A clean script still validates through the typed path.
        assert!(validate_script_typed(r#"fn on_candle() { "hold" }"#).is_ok());
    }

    // =========================================================================
    // AGT-606: resource-safety limits
    // =========================================================================

    // -----------------------------------------------------------------------
    // 15. test_configure_engine_limits_is_the_one_source_of_truth
    //
    // Locks in the constants `configure_engine_limits` applies, so a future edit
    // that changes a limit in only one of its four call sites (instead of editing
    // this shared function) shows up as a failing assertion here rather than as
    // silent drift between validate-time and run-time behavior.
    // -----------------------------------------------------------------------
    #[test]
    fn test_configure_engine_limits_is_the_one_source_of_truth() {
        let mut engine = Engine::new();
        configure_engine_limits(&mut engine);

        assert_eq!(engine.max_operations(), 1_000_000);
        assert_eq!(engine.max_call_levels(), 32);
        assert_eq!(engine.max_expr_depth(), 64);
        assert_eq!(engine.max_string_size(), 10_000);
        assert_eq!(engine.max_array_size(), MAX_ARRAY_SIZE);
        assert_eq!(engine.max_map_size(), MAX_MAP_SIZE);
    }

    // -----------------------------------------------------------------------
    // 16. test_max_array_size_enforced_via_push
    //
    // A script that keeps growing an array past MAX_ARRAY_SIZE via `push()` hits
    // Rhai's built-in data-size guard on every push (see `array_basic.rs::push`,
    // which explicitly checks `max_array_size` before completing). `on_candle`
    // must not panic — it should surface as an ordinary script error and return
    // Hold, exactly like any other Rhai error.
    // -----------------------------------------------------------------------
    #[test]
    fn test_max_array_size_enforced_via_push() {
        let script = r#"
fn on_candle() {
    let arr = [];
    for i in 0..20000 {
        arr.push(i);
    }
    "buy"
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "test_array_limit").unwrap();
        let candle = create_test_candle(dec!(1.1000), 0);
        let signal = strategy.on_candle(&candle);
        assert_eq!(signal, Signal::Hold, "array growth past the limit should abort to Hold, not panic or succeed");
    }

    // -----------------------------------------------------------------------
    // 17. test_max_array_size_literal_rejected_at_compile_time
    //
    // Rhai also enforces `max_array_size` against array *literals* at parse
    // time — a script cannot even compile if it writes out an oversized array
    // directly in source. `validate_script` must reject it (not panic, not
    // silently accept something the runtime path would then choke on).
    // -----------------------------------------------------------------------
    #[test]
    fn test_max_array_size_literal_rejected_at_compile_time() {
        let elements: String = (0..(MAX_ARRAY_SIZE + 10))
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let script = format!(
            r#"
fn on_candle() {{
    let arr = [{elements}];
    "buy"
}}
"#
        );
        let result = validate_script(&script);
        assert!(result.is_err(), "an array literal past MAX_ARRAY_SIZE should fail to compile");
    }

    // -----------------------------------------------------------------------
    // 18. test_max_map_size_enforced_when_touched_after_growth
    //
    // Unlike arrays' `push()`, Rhai's object-map index assignment (`m[k] = v`)
    // does not itself re-check container size on every insert — only the
    // *value being stored* is size-checked at that call site. The map's own
    // size is validated the next time the map is used as the receiver of a
    // built-in method call (e.g. `.len()`), which is exactly what a script
    // reading its own accumulated state back out would naturally do. This
    // test drives that realistic path rather than asserting something Rhai
    // itself doesn't actually check.
    // -----------------------------------------------------------------------
    #[test]
    fn test_max_map_size_enforced_when_touched_after_growth() {
        let script = r#"
fn on_candle() {
    let m = #{};
    for i in 0..5000 {
        m[to_string(i)] = i;
    }
    let n = m.len();
    "buy"
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "test_map_limit").unwrap();
        let candle = create_test_candle(dec!(1.1000), 0);
        let signal = strategy.on_candle(&candle);
        assert_eq!(signal, Signal::Hold, "map growth past the limit should abort to Hold, not panic or succeed");
    }

    // -----------------------------------------------------------------------
    // 19. test_max_map_size_literal_rejected_at_compile_time
    //
    // Same compile-time literal guard as arrays, for object maps.
    // -----------------------------------------------------------------------
    #[test]
    fn test_max_map_size_literal_rejected_at_compile_time() {
        let entries: String = (0..(MAX_MAP_SIZE + 10))
            .map(|i| format!("k{i}: {i}"))
            .collect::<Vec<_>>()
            .join(",");
        let script = format!(
            r#"
fn on_candle() {{
    let m = #{{{entries}}};
    "buy"
}}
"#
        );
        let result = validate_script(&script);
        assert!(result.is_err(), "a map literal past MAX_MAP_SIZE should fail to compile");
    }

    // -----------------------------------------------------------------------
    // 20. test_wall_clock_guard_terminates_a_stalled_script
    //
    // The daemon's real anti-hang guard (AC2): drives `register_wall_clock_guard`
    // directly rather than timing a real busy-loop, so the test is fast and
    // deterministic instead of racing against machine speed. Forces the shared
    // start time far enough into the past that the very first periodic check
    // (every WALL_CLOCK_CHECK_INTERVAL_OPS operations) already exceeds budget,
    // then confirms an infinite loop is terminated — not left to hang — and that
    // the termination reason is specifically the wall-clock guard's.
    // -----------------------------------------------------------------------
    #[test]
    fn test_wall_clock_guard_terminates_a_stalled_script() {
        let mut engine = Engine::new();
        let start = register_wall_clock_guard(&mut engine);

        // Simulate a call that has already been running far past budget. (Uses
        // `std::time::Duration` explicitly — this test module's `use super::*`
        // also pulls in `chrono::Duration` as the bare `Duration` name.)
        *start.lock().unwrap() = Instant::now() - std::time::Duration::from_secs(5);

        let result = engine.run("loop { let x = 1; }");

        let err = result.expect_err("a script that never returns must be terminated, not hang");
        let reason = wall_clock_terminated_reason(&err)
            .expect("termination should be attributable to the wall-clock guard");
        assert!(reason.contains("wall-clock budget"), "reason was: {reason}");
    }

    // -----------------------------------------------------------------------
    // 21. test_wall_clock_terminated_reason_ignores_unrelated_errors
    //
    // `wall_clock_terminated_reason` must only claim ordinary script errors (e.g.
    // a ZeroDivisionError) when they actually are `ErrorTerminated` from our guard
    // — otherwise `on_candle_extended`'s logging would mislabel unrelated script
    // bugs as wall-clock timeouts.
    // -----------------------------------------------------------------------
    #[test]
    fn test_wall_clock_terminated_reason_ignores_unrelated_errors() {
        let engine = Engine::new();
        let result = engine.run("let x = 1 / 0;");
        let err = result.expect_err("division by zero should be a script error");
        assert!(wall_clock_terminated_reason(&err).is_none());
    }

    // -----------------------------------------------------------------------
    // 22. test_take_abort_event_fires_once_then_reset_rearms_it
    //
    // AC4: the consecutive-error abort threshold must emit an explicit,
    // edge-triggered health event distinguishable from a legitimate "no signal"
    // Hold — not a silent Hold-forever fallback.
    // -----------------------------------------------------------------------
    #[test]
    fn test_take_abort_event_fires_once_then_reset_rearms_it() {
        let script = r#"
fn on_candle() {
    let x = 1 / 0;
    "buy"
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "test_abort_event").unwrap();
        let candle = create_test_candle(dec!(1.1000), 0);

        // Before the threshold: no event, ordinary Hold.
        for _ in 0..MAX_CONSECUTIVE_ERRORS - 1 {
            assert_eq!(strategy.on_candle(&candle), Signal::Hold);
            assert!(!strategy.take_abort_event());
        }

        // The candle that trips the threshold: event fires exactly once.
        assert_eq!(strategy.on_candle(&candle), Signal::Hold);
        assert!(strategy.take_abort_event(), "abort event should fire the candle the threshold is hit");
        assert!(!strategy.take_abort_event(), "abort event must not re-fire every subsequent Hold candle");
        assert!(strategy.abort_reason().contains("consecutive"));

        // Further candles stay silently Hold (no event) until reset() re-arms it.
        assert_eq!(strategy.on_candle(&candle), Signal::Hold);
        assert!(!strategy.take_abort_event());

        strategy.reset();
        assert!(!strategy.take_abort_event(), "reset() must not itself report an abort event");
    }

    // -----------------------------------------------------------------------
    // 23. test_script_stays_permanently_held_after_abort_until_reset
    //
    // Once the threshold trips, `on_candle_extended`'s early-return guard means
    // the script body never runs again — that's the pre-existing "Hold forever"
    // behavior described in the AGT-606 ticket. This ticket doesn't change that
    // behavior; it makes it *observable*: `take_abort_event()` must report the
    // abort exactly once, not on every one of the (now permanent) Hold candles
    // that follow, and only `reset()` re-arms the strategy.
    // -----------------------------------------------------------------------
    #[test]
    fn test_script_stays_permanently_held_after_abort_until_reset() {
        let script = r#"
let call_count = 0;

fn on_candle() {
    call_count += 1;
    let x = 1 / 0;
    "buy"
}
"#;
        let mut strategy = ScriptedStrategy::from_script(script, "test_permanent_hold").unwrap();
        let candle = create_test_candle(dec!(1.1000), 0);

        for _ in 0..MAX_CONSECUTIVE_ERRORS {
            assert_eq!(strategy.on_candle(&candle), Signal::Hold);
        }
        assert!(strategy.take_abort_event(), "abort event should fire once the threshold is hit");

        // Permanently Hold afterward, with no repeat health events.
        for _ in 0..10 {
            assert_eq!(strategy.on_candle(&candle), Signal::Hold);
            assert!(!strategy.take_abort_event(), "must not re-report an abort on every subsequent Hold candle");
        }

        // reset() re-arms the strategy (and the abort trigger) for a fresh run.
        strategy.reset();
        assert_eq!(strategy.on_candle(&candle), Signal::Hold, "script fails immediately again post-reset");
        assert!(!strategy.take_abort_event(), "a single failure after reset is not yet at the threshold");
    }

}
