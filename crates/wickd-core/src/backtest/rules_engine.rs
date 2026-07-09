//! Rules Evaluation Engine
//!
//! Evaluates entry and exit rules against indicator outputs to generate trading signals.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal_macros::dec;

use std::collections::{HashMap, VecDeque};
use crate::models::Candle;
use super::indicator_engine::IndicatorEngine;
use super::strategy::{Signal, PendingOrderInfo};
use super::pivots::{PivotConfig, PivotLevels, PivotPeriodTracker};
use super::regime_detector::{RegimeDetector, RegimeConfig};
use super::mtf::MtfCandleStore;

// Re-export all types from rules_types for backward compatibility
pub use super::rules_types::*;

// ============================================================================
// Rules Engine
// ============================================================================

pub struct RulesEngine {
    pub(crate) strategy: StrategyDefinition,
    pub(crate) indicator_engine: IndicatorEngine,
    pub(crate) position: Option<PositionState>,
    pub(crate) price_history: Vec<Candle>,
    max_price_history: usize,
    // Risk tracking
    starting_balance: Decimal,
    current_balance: Decimal,
    daily_pnl: Decimal,
    // S/R Zones for zone-based triggers
    pub(crate) sr_zones: Vec<SRZone>,
    // Pivot point tracking
    pivot_config: Option<PivotConfig>,
    pub(crate) current_pivots: Option<PivotLevels>,
    pivot_tracker: PivotPeriodTracker,
    // Resolved parameter values for this run
    pub(crate) resolved_params: std::collections::HashMap<String, f64>,
    // Price action pattern detection
    pub(crate) regime_detector: RegimeDetector,
    /// Current candle index in backtest (for pattern lookback)
    current_candle_index: usize,
    /// Whether patterns have been detected (requires prepare_for_backtest call)
    patterns_detected: bool,
    /// Pip value for this instrument (e.g., 0.0001 for standard forex, 0.01 for JPY/XAU)
    pip_value: Decimal,
    /// Whether we've warned about time triggers in live mode (to avoid log spam)
    warned_time_triggers_live: bool,
    // Multi-timeframe (MTF) support
    /// Per-HTF-timeframe indicator engines (e.g., "D" -> IndicatorEngine for daily indicators)
    pub(crate) htf_indicator_engines: HashMap<String, IndicatorEngine>,
    /// Pre-fetched HTF candle store with timestamp-based advancement
    pub(crate) mtf_candle_store: MtfCandleStore,
    /// Per-HTF-timeframe price histories for HTF price data source resolution
    pub(crate) htf_price_histories: HashMap<String, VecDeque<Candle>>,
}

impl RulesEngine {
    /// Create a new RulesEngine with default parameter values
    pub fn new(strategy: StrategyDefinition) -> Result<Self, String> {
        Self::with_params(strategy, None)
    }

    /// Create a new RulesEngine with optional parameter overrides
    pub fn with_params(
        strategy: StrategyDefinition,
        param_overrides: Option<std::collections::HashMap<String, f64>>,
    ) -> Result<Self, String> {
        // Build resolved params from defaults, with optional overrides
        // This must happen BEFORE creating indicator engine so params can be resolved
        let mut resolved_params = std::collections::HashMap::new();
        for param in &strategy.parameters {
            let value = param_overrides
                .as_ref()
                .and_then(|o| o.get(&param.id))
                .copied()
                .unwrap_or(param.default);
            resolved_params.insert(param.id.clone(), value);
        }

        // Create indicator engine with resolved params for parameterized indicator configs
        tracing::debug!(
            "Creating RulesEngine with {} indicators, {} parameters",
            strategy.indicators.len(),
            strategy.parameters.len()
        );
        for ind in &strategy.indicators {
            tracing::debug!("  Indicator: id={}, type={}", ind.id, ind.indicator_type);
        }

        // Log any disabled conditions for debugging
        for (rule_idx, rule) in strategy.entry_rules.iter().enumerate() {
            for (cond_idx, cond) in rule.conditions.iter().enumerate() {
                if cond.disabled.is_some() {
                    tracing::info!(
                        "[RulesEngine] Entry rule {} condition {} has disabled={:?}",
                        rule_idx, cond_idx, cond.disabled
                    );
                }
            }
        }

        // Separate indicators by timeframe for MTF support
        let primary_indicators: Vec<_> = strategy.indicators.iter()
            .filter(|ind| ind.timeframe.is_none())
            .cloned()
            .collect();

        let mut htf_groups: HashMap<String, Vec<shared::IndicatorConfig>> = HashMap::new();
        for ind in &strategy.indicators {
            if let Some(ref tf) = ind.timeframe {
                htf_groups.entry(tf.clone()).or_default().push(ind.clone());
            }
        }

        // Create primary indicator engine from primary-timeframe indicators only
        let indicator_engine = IndicatorEngine::from_config_with_params(
            &primary_indicators,
            100,
            &resolved_params,
        )?;

        // Create per-timeframe HTF indicator engines
        let mut htf_indicator_engines = HashMap::new();
        for (tf, configs) in &htf_groups {
            let engine = IndicatorEngine::from_config_with_params(
                configs,
                100,
                &resolved_params,
            )?;
            htf_indicator_engines.insert(tf.clone(), engine);
        }

        // Initialize HTF price histories
        let mut htf_price_histories = HashMap::new();
        for tf in htf_groups.keys() {
            htf_price_histories.insert(tf.clone(), VecDeque::new());
        }

        if !htf_indicator_engines.is_empty() {
            tracing::info!(
                "[RulesEngine] Created {} HTF indicator engines for timeframes: {:?}",
                htf_indicator_engines.len(),
                htf_indicator_engines.keys().collect::<Vec<_>>()
            );
        }

        Ok(Self {
            strategy,
            indicator_engine,
            position: None,
            price_history: Vec::new(),
            max_price_history: 100,
            starting_balance: dec!(10000),
            current_balance: dec!(10000),
            daily_pnl: Decimal::ZERO,
            sr_zones: Vec::new(),
            pivot_config: None,
            current_pivots: None,
            pivot_tracker: PivotPeriodTracker::new(),
            resolved_params,
            regime_detector: RegimeDetector::new(RegimeConfig::default()),
            current_candle_index: 0,
            patterns_detected: false,
            pip_value: dec!(0.0001), // Default for standard forex pairs
            warned_time_triggers_live: false,
            htf_indicator_engines,
            mtf_candle_store: MtfCandleStore::new(),
            htf_price_histories,
        })
    }

    /// Get the resolved parameter values
    pub fn get_resolved_params(&self) -> &std::collections::HashMap<String, f64> {
        &self.resolved_params
    }

    /// Set S/R zones for zone-based trigger evaluation
    pub fn set_sr_zones(&mut self, zones: Vec<SRZone>) {
        self.sr_zones = zones;
    }

    /// Set pivot point configuration
    pub fn set_pivot_config(&mut self, config: PivotConfig) {
        self.pivot_config = Some(config);
    }

    /// Get current pivot levels (if calculated)
    pub fn get_current_pivots(&self) -> Option<&PivotLevels> {
        self.current_pivots.as_ref()
    }

    /// Set pip value for the instrument being traded.
    /// This affects fixed-pip stop loss calculations.
    ///
    /// Common pip values:
    /// - Standard forex (EURUSD, GBPUSD, etc.): 0.0001
    /// - JPY pairs (USDJPY, EURJPY, etc.): 0.01
    /// - Gold (XAUUSD): 0.01
    /// - Silver (XAGUSD): 0.001
    pub fn set_pip_value(&mut self, pip_value: Decimal) {
        self.pip_value = pip_value;
    }

    /// Set pip value based on instrument name.
    /// Automatically determines the correct pip value for the instrument.
    pub fn set_pip_value_for_instrument(&mut self, instrument: &str) {
        self.pip_value = shared::get_pip_value(instrument);
        // Also update the regime detector config
        self.regime_detector = RegimeDetector::new(RegimeConfig::for_instrument(instrument));
    }

    /// Reclassify indicators whose explicit timeframe matches the chart's primary
    /// granularity. These belong in the primary indicator engine (fed every candle),
    /// not in an HTF engine that relies on MtfCandleStore.
    ///
    /// Must be called BEFORE `set_mtf_candle_store` so that the HTF engine for the
    /// primary timeframe is already removed when the store seeds initial candles.
    pub fn set_primary_granularity(&mut self, granularity: &str) {
        // Remove the HTF engine for this timeframe (if any)
        let removed_engine = self.htf_indicator_engines.remove(granularity);
        if removed_engine.is_none() {
            return; // No indicators on this timeframe, nothing to do
        }

        // Move matching indicators from HTF to primary engine
        for config in &self.strategy.indicators {
            if config.timeframe.as_deref() == Some(granularity) {
                let params = config.resolve_params(&self.resolved_params);
                match super::indicator_engine::create_indicator(config.indicator_type, &params) {
                    Ok(indicator) => {
                        self.indicator_engine.add_indicator(&config.id, indicator);
                        tracing::info!(
                            "[RulesEngine] Reclassified indicator '{}' ({}) from HTF '{}' to primary engine",
                            config.id, config.indicator_type, granularity
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[RulesEngine] Failed to reclassify indicator '{}': {}",
                            config.id, e
                        );
                    }
                }
            }
        }

        // Clean up the HTF price history for this timeframe too
        self.htf_price_histories.remove(granularity);
    }

    /// Set the MTF candle store (populated by command handlers after fetching HTF candles)
    pub fn set_mtf_candle_store(&mut self, store: MtfCandleStore) {
        // Seed HTF price histories and indicator engines with the initial candle (index 0)
        // so that HTF price/indicator data is available before the first HTF candle transition
        for tf in store.timeframes() {
            if let Some(initial_candle) = store.current_candle(tf) {
                let candle = initial_candle.clone();
                if let Some(engine) = self.htf_indicator_engines.get_mut(tf.as_str()) {
                    engine.on_candle(&candle);
                }
                if let Some(history) = self.htf_price_histories.get_mut(tf.as_str()) {
                    history.push_back(candle);
                }
            }
        }
        self.mtf_candle_store = store;
    }

    /// Append a newly completed HTF candle to the store.
    /// The candle will be picked up by `advance_htf_engines` on the next primary candle.
    pub fn append_htf_candle(&mut self, timeframe: &str, candle: Candle) {
        self.mtf_candle_store.append_candle(timeframe, candle);
    }

    /// Get the HTF indicator engines (for cloning in optimizer)
    pub fn htf_indicator_engines(&self) -> &HashMap<String, IndicatorEngine> {
        &self.htf_indicator_engines
    }

    /// Prepare for backtest by detecting price action patterns from all candles.
    /// This MUST be called before the main backtest loop to enable price action
    /// regime detection (gaps, order blocks, supply/demand zones, structure retests).
    ///
    /// If not called, price action regimes will return false (no patterns detected).
    pub fn prepare_for_backtest(&mut self, candles: &[Candle]) {
        // Calculate ATR values for pattern detection
        let atr_values = self.calculate_atr_series(candles, 14);

        // Set S/R zones on the regime detector if we have any
        self.regime_detector.set_sr_zones(self.sr_zones.clone());

        // Detect all price action patterns
        self.regime_detector.detect_patterns(candles, &atr_values);
        self.patterns_detected = true;

        tracing::debug!(
            "Prepared for backtest: detected patterns in {} candles",
            candles.len()
        );
    }

    /// Calculate ATR series for pattern detection
    fn calculate_atr_series(&self, candles: &[Candle], period: usize) -> Vec<Decimal> {
        if candles.is_empty() {
            return Vec::new();
        }

        let mut atr_values = Vec::with_capacity(candles.len());
        let mut tr_sum = Decimal::ZERO;

        for (i, candle) in candles.iter().enumerate() {
            // Calculate true range
            let tr = if i == 0 {
                candle.mid.high - candle.mid.low
            } else {
                let prev_close = candles[i - 1].mid.close;
                let h_l = candle.mid.high - candle.mid.low;
                let h_pc = (candle.mid.high - prev_close).abs();
                let l_pc = (candle.mid.low - prev_close).abs();
                h_l.max(h_pc).max(l_pc)
            };

            if i < period {
                tr_sum += tr;
                if i == period - 1 {
                    atr_values.push(tr_sum / Decimal::from(period));
                } else {
                    // Not enough data yet, use simple average of what we have
                    atr_values.push(tr_sum / Decimal::from(i + 1));
                }
            } else {
                // EMA-style smoothing: ATR = (prev_atr * (period - 1) + TR) / period
                let prev_atr = atr_values[i - 1];
                let new_atr = (prev_atr * Decimal::from(period - 1) + tr) / Decimal::from(period);
                atr_values.push(new_atr);
            }
        }

        atr_values
    }

    /// Set the starting and current balance for risk tracking
    pub fn set_balance(&mut self, balance: Decimal) {
        self.starting_balance = balance;
        self.current_balance = balance;
    }

    /// Update the current balance and daily P&L after a trade
    pub fn update_balance(&mut self, pnl: Decimal) {
        self.current_balance += pnl;
        self.daily_pnl += pnl;
    }

    /// Reset daily P&L (call at start of new trading day)
    pub fn reset_daily_pnl(&mut self) {
        self.daily_pnl = Decimal::ZERO;
    }

    /// Process a candle for warmup only - updates indicators and price history
    /// but does NOT evaluate entry/exit rules or modify position state.
    /// Use this during indicator warmup to avoid generating spurious signals.
    pub fn warmup_candle(&mut self, candle: &Candle) {
        // Store price history
        self.price_history.push(candle.clone());
        if self.price_history.len() > self.max_price_history {
            self.price_history.remove(0);
        }

        // Advance HTF candle stores and feed new candles to HTF indicator engines
        self.advance_htf_engines(candle);

        // Update all indicators
        self.indicator_engine.on_candle(candle);

        // Update pivot point tracker (even during warmup to build up previous period data)
        self.update_pivots(candle);

        // Update regime detector's current index and ATR (even during warmup)
        self.regime_detector.set_current_index(self.current_candle_index);
        if let Some(atr) = self.get_indicator_value_by_type(IndicatorType::Atr, "value") {
            self.regime_detector.update_atr(atr);
        }
        self.current_candle_index += 1;
    }

    /// Process a candle and return the trading signal
    ///
    /// NOTE: This method is for backtesting and tracks internal position state.
    /// For live trading, use `on_candle_live` instead.
    pub fn on_candle(&mut self, candle: &Candle) -> RulesSignal {
        // Store price history
        self.price_history.push(candle.clone());
        if self.price_history.len() > self.max_price_history {
            self.price_history.remove(0);
        }

        // Advance HTF candle stores and feed new candles to HTF indicator engines
        self.advance_htf_engines(candle);

        // Update all indicators
        self.indicator_engine.on_candle(candle);

        // Update pivot point tracker
        self.update_pivots(candle);

        // Update regime detector's current index and ATR
        self.regime_detector.set_current_index(self.current_candle_index);
        if let Some(atr) = self.get_indicator_value_by_type(IndicatorType::Atr, "value") {
            self.regime_detector.update_atr(atr);
        }
        self.current_candle_index += 1;

        // Update position state
        if let Some(ref mut pos) = self.position {
            pos.bars_since_entry += 1;
        }

        // Update trailing values for captured indicators/prices (separate borrow scope)
        self.update_trailing_values(candle);

        // Update trailing stop loss if using variable source with trailing evaluation
        self.update_trailing_stop_loss(candle);

        // Evaluate rules based on current position
        if self.position.is_some() {
            self.evaluate_exit_rules(candle)
        } else {
            self.evaluate_entry_rules(candle)
        }
    }

    /// Process a candle for live trading - does NOT track internal position state.
    ///
    /// The `position_direction` parameter should come from the actual broker (e.g., OANDA).
    /// - `None` means no open position
    /// - `Some(Long)` means user has a long position
    /// - `Some(Short)` means user has a short position
    ///
    /// When position exists:
    /// - First check exit rules for that direction (exit takes priority)
    /// - If no exit, check entry rules (allows scaling in / adding to positions)
    ///
    /// When no position:
    /// - Only check entry rules
    pub fn on_candle_live(&mut self, candle: &Candle, position_direction: Option<PositionDirection>) -> RulesSignal {
        // Warn once about time-based triggers not working in live mode
        if !self.warned_time_triggers_live {
            let has_time_triggers = self.strategy.exit_rules.iter().any(|rule| {
                rule.conditions.iter().any(|cond| {
                    matches!(&cond.primary.trigger, Trigger::Time(_))
                        || cond.chain.iter().any(|c| matches!(&c.trigger.trigger, Trigger::Time(_)))
                })
            });
            if has_time_triggers {
                tracing::warn!(
                    "[RulesEngine] Strategy contains time-based exit triggers which are NOT supported in live trading mode. \
                    Time triggers require position entry time tracking which is only available in backtesting. \
                    Consider using other exit conditions for live trading."
                );
                self.warned_time_triggers_live = true;
            }
        }

        // Store price history
        self.price_history.push(candle.clone());
        if self.price_history.len() > self.max_price_history {
            self.price_history.remove(0);
        }

        // Advance HTF candle stores and feed new candles to HTF indicator engines
        self.advance_htf_engines(candle);

        // Update all indicators
        self.indicator_engine.on_candle(candle);

        // Update pivot point tracker
        self.update_pivots(candle);

        tracing::info!(
            "[RulesEngine] on_candle_live: position_direction={:?}, candle_time={}",
            position_direction,
            candle.time
        );

        // Evaluate rules based on actual broker position state
        if let Some(direction) = position_direction {
            // Check exit rules first for this direction (exit takes priority)
            let exit_signal = self.evaluate_exit_rules_live(candle, direction);
            if exit_signal != RulesSignal::Hold {
                tracing::info!("[RulesEngine] on_candle_live: returning exit signal: {:?}", exit_signal);
                return exit_signal;
            }
            // If no exit, check entry rules (for scaling in / adding to position)
            self.evaluate_entry_rules_live(candle)
        } else {
            // No position - only check entry rules
            self.evaluate_entry_rules_live(candle)
        }
    }

    /// Evaluate exit rules for live trading with a known position direction
    /// Unlike the backtest version, this doesn't require internal position state
    fn evaluate_exit_rules_live(&self, candle: &Candle, position_direction: PositionDirection) -> RulesSignal {
        // Sort exit rules by priority (higher first)
        let mut sorted_rules: Vec<ExitRule> = self.strategy.exit_rules.to_vec();
        sorted_rules.sort_by(|a, b| b.priority.cmp(&a.priority));

        tracing::info!(
            "[RulesEngine] evaluate_exit_rules_live: position={:?}, exit_rules_count={}, candle O={} C={}",
            position_direction,
            sorted_rules.len(),
            candle.mid.open,
            candle.mid.close
        );

        for rule in &sorted_rules {
            // Check if this exit rule applies to the position direction
            let applies_to_position = rule.direction.applies_to(position_direction);
            tracing::info!(
                "[RulesEngine] Exit rule '{}': direction={:?}, applies_to_position={:?}",
                rule.name.as_deref().unwrap_or(&rule.id),
                rule.direction,
                applies_to_position
            );

            if !applies_to_position {
                continue;
            }

            // Evaluate the exit rule conditions
            let triggered = self.evaluate_exit_conditions_live(&rule.conditions, candle, position_direction);
            tracing::info!(
                "[RulesEngine] Exit rule '{}': conditions_triggered={}",
                rule.name.as_deref().unwrap_or(&rule.id),
                triggered
            );

            if triggered {
                // Resolve close_percent (defaults to 100% if not specified or invalid)
                let close_percent = self.resolve_parameterized(&rule.close_percent)
                    .and_then(|d| d.to_f64())
                    .unwrap_or(100.0);

                if close_percent >= 100.0 {
                    return RulesSignal::Exit {
                        reason: rule.name.clone().unwrap_or_else(|| rule.id.clone()),
                        close_percent: 100.0,
                    };
                } else {
                    return RulesSignal::PartialExit {
                        reason: rule.name.clone().unwrap_or_else(|| rule.id.clone()),
                        close_percent,
                        new_stop_loss: None, // Live trading doesn't track internal SL
                    };
                }
            }
        }

        RulesSignal::Hold
    }

    /// Evaluate exit conditions for live trading
    /// Uses the existing evaluate_condition method that handles all trigger types
    fn evaluate_exit_conditions_live(&self, conditions: &[Condition], candle: &Candle, _position_direction: PositionDirection) -> bool {
        // All conditions must be met (AND logic)
        for condition in conditions {
            if !self.evaluate_condition(condition, candle) {
                return false;
            }
        }
        !conditions.is_empty()
    }

    /// Evaluate entry rules without modifying internal position state
    /// Used for live trading where position state comes from the broker
    fn evaluate_entry_rules_live(&mut self, candle: &Candle) -> RulesSignal {
        self.evaluate_entry_rules_v2(candle)
    }

    /// Check whether a stop loss is on the correct side of entry price.
    /// For longs, SL must be strictly below entry. For shorts, strictly above.
    fn is_sl_valid(direction: PositionDirection, stop_loss: Decimal, entry_price: Decimal) -> bool {
        match direction {
            PositionDirection::Long => stop_loss < entry_price,
            PositionDirection::Short => stop_loss > entry_price,
        }
    }

    /// Calculate stop loss and take profit for a signal without opening a position.
    /// Returns None if the stop loss is on the wrong side of entry (e.g., swing low above
    /// entry for a long), which means the trade should be skipped.
    pub(crate) fn calculate_sl_tp_for_signal(&self, direction: PositionDirection, candle: &Candle) -> Option<(Decimal, Decimal)> {
        let entry_price = candle.mid.close;
        let stop_loss = self.calculate_stop_loss(direction, candle);

        if !Self::is_sl_valid(direction, stop_loss, entry_price) {
            tracing::warn!(
                "Skipping trade: stop loss ({}) is on wrong side of entry ({}) for {:?}",
                stop_loss, entry_price, direction
            );
            return None;
        }

        let risk = (entry_price - stop_loss).abs();

        // Defense-in-depth: reject zero-risk trades even if SL side check passed
        if risk.is_zero() {
            tracing::warn!("Skipping trade: zero risk (SL == entry) for {:?}", direction);
            return None;
        }

        let rr_ratio = self.get_rr_ratio(direction);

        let take_profit = match direction {
            PositionDirection::Long => entry_price + risk * rr_ratio,
            PositionDirection::Short => entry_price - risk * rr_ratio,
        };

        Some((stop_loss, take_profit))
    }

    fn evaluate_entry_rules(&mut self, candle: &Candle) -> RulesSignal {
        self.evaluate_entry_rules_v2_with_position(candle)
    }

    pub(crate) fn open_position(&mut self, direction: PositionDirection, candle: &Candle) {
        let entry_price = candle.mid.close;

        // Calculate stop loss based on custom source or default (Chandelier/ATR)
        let stop_loss = self.calculate_stop_loss(direction, candle);

        // Validate SL is on the correct side of entry — skip if invalid
        if !Self::is_sl_valid(direction, stop_loss, entry_price) {
            tracing::warn!(
                "Skipping position open: stop loss ({}) is on wrong side of entry ({}) for {:?}",
                stop_loss, entry_price, direction
            );
            return;
        }

        // Calculate take profit based on R:R ratio (uses direction-specific ratio if set)
        let risk = (entry_price - stop_loss).abs();
        let rr_ratio = self.get_rr_ratio(direction);

        let take_profit = match direction {
            PositionDirection::Long => entry_price + risk * rr_ratio,
            PositionDirection::Short => entry_price - risk * rr_ratio,
        };

        // Capture indicator/price values at entry for exit rules with capture=at_entry
        let mut captured_values = self.capture_values_at_entry(direction, candle);

        // Also capture the stop loss source value if it's an indicator
        // Use direction-specific source for shorts if available
        let sl_source = match direction {
            PositionDirection::Short => {
                self.strategy.risk_settings.stop_loss_source_short.as_ref()
                    .or(self.strategy.risk_settings.stop_loss_source.as_ref())
            }
            PositionDirection::Long => {
                self.strategy.risk_settings.stop_loss_source.as_ref()
            }
        };
        if let Some(StopLossSource::Indicator { indicator, output, .. }) = sl_source {
            if let Some(value) = self.indicator_engine.get_latest(indicator, output) {
                let key = format!("_sl_source.{}.{}", indicator, output);
                captured_values.insert(key, CapturedValue {
                    initial_value: value,
                    current_value: value,
                    trail_config: None,
                });
            }
        }

        self.position = Some(PositionState {
            direction,
            entry_price,
            stop_loss,
            take_profit,
            remaining_percent: 100.0,
            bars_since_entry: 0,
            triggered_partials: Vec::new(),
            captured_values,
            entry_time: candle.time,
        });
    }

    /// Capture values from exit rules that have capture=at_entry
    fn capture_values_at_entry(
        &self,
        direction: PositionDirection,
        candle: &Candle,
    ) -> std::collections::HashMap<String, CapturedValue> {
        let mut captured = std::collections::HashMap::new();

        for rule in &self.strategy.exit_rules {
            if !rule.direction.applies_to(direction) {
                continue;
            }

            // Collect triggers from new conditions format
            if !rule.conditions.is_empty() {
                for condition in &rule.conditions {
                    self.capture_from_trigger(&condition.primary.trigger, candle, &mut captured);
                    for chained in &condition.chain {
                        self.capture_from_trigger(&chained.trigger.trigger, candle, &mut captured);
                    }
                }
            } else if let Some(ref chain) = rule.trigger_chain {
                // Fall back to legacy trigger_chain
                self.capture_from_trigger(&chain.primary, candle, &mut captured);
                for chained in &chain.chain {
                    self.capture_from_trigger(&chained.trigger, candle, &mut captured);
                }
            }
        }

        captured
    }

    /// Extract and capture data sources from a trigger that have capture=at_entry
    fn capture_from_trigger(
        &self,
        trigger: &Trigger,
        candle: &Candle,
        captured: &mut std::collections::HashMap<String, CapturedValue>,
    ) {
        match trigger {
            Trigger::Cross(t) => {
                self.maybe_capture_source(&t.left, candle, captured);
                self.maybe_capture_source(&t.right, candle, captured);
            }
            Trigger::Compare(t) => {
                self.maybe_capture_source(&t.left, candle, captured);
                self.maybe_capture_source(&t.right, candle, captured);
            }
            Trigger::Threshold(t) => {
                self.maybe_capture_source(&t.source, candle, captured);
            }
            // Other trigger types don't have data sources to capture
            _ => {}
        }
    }

    /// If a data source has capture=at_entry, resolve and store its value
    fn maybe_capture_source(
        &self,
        source: &DataSource,
        candle: &Candle,
        captured: &mut std::collections::HashMap<String, CapturedValue>,
    ) {
        let (capture_mode, trail_config, key) = match source {
            DataSource::Indicator(src) => {
                if src.capture == CaptureMode::AtEntry {
                    let key = format!("indicator.{}.{}", src.indicator, src.output);
                    (true, src.trail.clone(), key)
                } else {
                    return;
                }
            }
            DataSource::Price(src) => {
                if src.capture == CaptureMode::AtEntry {
                    let key = format!("price.{}", src.value);
                    (true, src.trail.clone(), key)
                } else {
                    return;
                }
            }
            // Other source types don't support capture
            _ => return,
        };

        if !capture_mode {
            return;
        }

        // Resolve the current value
        if let Some(value) = self.resolve_data_source_v2(source, candle, 0) {
            captured.insert(
                key,
                CapturedValue {
                    initial_value: value,
                    current_value: value,
                    trail_config,
                },
            );
        }
    }

    /// Update captured values with trailing logic.
    /// For trailing stops, this moves the stop level in the favorable direction.
    fn update_trailing_values(&mut self, candle: &Candle) {
        let pos = match &mut self.position {
            Some(p) => p,
            None => return,
        };

        // Collect keys to update (can't iterate and mutate simultaneously)
        let keys_to_update: Vec<String> = pos
            .captured_values
            .iter()
            .filter_map(|(key, cv)| {
                if let Some(ref trail) = cv.trail_config {
                    if trail.enabled {
                        return Some(key.clone());
                    }
                }
                None
            })
            .collect();

        let direction = pos.direction;

        for key in keys_to_update {
            // Get current indicator/price value
            let current_value = if key.starts_with("indicator.") {
                // Parse "indicator.name.output"
                let parts: Vec<&str> = key.splitn(3, '.').collect();
                if parts.len() == 3 {
                    self.indicator_engine.get_output(parts[1], parts[2], 0)
                } else {
                    continue;
                }
            } else if key.starts_with("price.") {
                // Parse "price.close" etc.
                let price_type_str = &key[6..];
                let price_type = match price_type_str {
                    "open" => PriceType::Open,
                    "high" => PriceType::High,
                    "low" => PriceType::Low,
                    "close" => PriceType::Close,
                    _ => continue, // Unknown price type, skip
                };
                Some(self.get_price_value(price_type, candle))
            } else {
                continue;
            };

            if let Some(new_value) = current_value {
                if let Some(cv) = self.position.as_mut().and_then(|p| p.captured_values.get_mut(&key)) {
                    let trail_percent = cv.trail_config.as_ref().and_then(|t| t.percent).unwrap_or(0.0);

                    // Trail logic: move the level in favorable direction based on position direction
                    // For a long position stop loss: trail UP (higher stop = more protection)
                    // For a short position stop loss: trail DOWN (lower stop = more protection)
                    match direction {
                        PositionDirection::Long => {
                            // Trail up: if new value is higher, update
                            // With trail percent: only trail if new value is within X% of initial
                            if trail_percent > 0.0 {
                                // Constrained trailing: don't exceed initial + trail_percent
                                let max_trail = cv.initial_value * (Decimal::ONE + Decimal::try_from(trail_percent / 100.0).unwrap_or(Decimal::ZERO));
                                let new_capped = new_value.min(max_trail);
                                if new_capped > cv.current_value {
                                    cv.current_value = new_capped;
                                }
                            } else {
                                // Simple trailing: follow the level if it goes up
                                if new_value > cv.current_value {
                                    cv.current_value = new_value;
                                }
                            }
                        }
                        PositionDirection::Short => {
                            // Trail down: if new value is lower, update
                            if trail_percent > 0.0 {
                                let min_trail = cv.initial_value * (Decimal::ONE - Decimal::try_from(trail_percent / 100.0).unwrap_or(Decimal::ZERO));
                                let new_capped = new_value.max(min_trail);
                                if new_capped < cv.current_value {
                                    cv.current_value = new_capped;
                                }
                            } else {
                                if new_value < cv.current_value {
                                    cv.current_value = new_value;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Update trailing stop loss if using variable source with trailing evaluation mode.
    /// Only trails in the favorable direction (higher for longs, lower for shorts).
    fn update_trailing_stop_loss(&mut self, candle: &Candle) {
        // Check if we have a position and a variable stop loss source with trailing
        let (direction, current_sl) = match &self.position {
            Some(pos) => (pos.direction, pos.stop_loss),
            None => return,
        };

        // Use direction-specific stop loss source for shorts if available
        let sl_source = match direction {
            PositionDirection::Short => {
                self.strategy.risk_settings.stop_loss_source_short.as_ref()
                    .or(self.strategy.risk_settings.stop_loss_source.as_ref())
            }
            PositionDirection::Long => {
                self.strategy.risk_settings.stop_loss_source.as_ref()
            }
        };

        // Check if stop loss source is a variable with trailing evaluation
        let variable_id = match sl_source {
            Some(StopLossSource::Variable { variable, evaluation }) => {
                if *evaluation == StopLossEvaluationMode::Trailing {
                    variable.clone()
                } else {
                    return; // at_open mode - don't trail
                }
            }
            _ => return, // Not a variable source
        };

        // Find and evaluate the variable
        let new_sl: Option<Decimal> = self.strategy.variables.iter()
            .find(|v| v.id == variable_id)
            .and_then(|var_def| self.resolve_variable_expression(&var_def.expression, candle, 0));

        let new_sl = match new_sl {
            Some(v) => v,
            None => return, // Variable evaluation failed
        };

        // Update stop loss only in favorable direction
        if let Some(ref mut pos) = self.position {
            match direction {
                PositionDirection::Long => {
                    // For longs, trail UP (higher stop = better protection)
                    if new_sl > current_sl {
                        pos.stop_loss = new_sl;
                    }
                }
                PositionDirection::Short => {
                    // For shorts, trail DOWN (lower stop = better protection)
                    if new_sl < current_sl {
                        pos.stop_loss = new_sl;
                    }
                }
            }
        }
    }

    fn calculate_stop_loss(&self, direction: PositionDirection, candle: &Candle) -> Decimal {
        // Use direction-specific stop loss source if available for shorts
        let source = match direction {
            PositionDirection::Short => {
                self.strategy.risk_settings.stop_loss_source_short.as_ref()
                    .or(self.strategy.risk_settings.stop_loss_source.as_ref())
            }
            PositionDirection::Long => {
                self.strategy.risk_settings.stop_loss_source.as_ref()
            }
        };

        // Check for custom stop loss source
        if let Some(source) = source {
            match source {
                StopLossSource::Indicator { indicator, output, .. } => {
                    // Get the indicator value as the stop level
                    if let Some(value) = self.indicator_engine.get_latest(indicator, output) {
                        return value;
                    }
                    // Fall through to default if indicator not found
                }
                StopLossSource::FixedPips { pips } => {
                    let pips_val = self.resolve_parameterized(pips).unwrap_or(dec!(50));
                    return match direction {
                        PositionDirection::Long => candle.mid.close - pips_val * self.pip_value,
                        PositionDirection::Short => candle.mid.close + pips_val * self.pip_value,
                    };
                }
                StopLossSource::Percent { percent } => {
                    let pct = self.resolve_parameterized(percent).unwrap_or(dec!(2)) / dec!(100);
                    return match direction {
                        PositionDirection::Long => candle.mid.close * (Decimal::ONE - pct),
                        PositionDirection::Short => candle.mid.close * (Decimal::ONE + pct),
                    };
                }
                StopLossSource::Variable { variable, .. } => {
                    // Find the variable definition and evaluate it
                    if let Some(var_def) = self.strategy.variables.iter().find(|v| v.id == *variable) {
                        if let Some(value) = self.resolve_variable_expression(&var_def.expression, candle, 0) {
                            return value;
                        }
                    }
                    // Fall through to default if variable not found
                }
            }
        }

        // Default: Use fixed percentage (2%)
        let pct = dec!(0.02);
        match direction {
            PositionDirection::Long => candle.mid.close * (dec!(1) - pct),
            PositionDirection::Short => candle.mid.close * (dec!(1) + pct),
        }
    }

    #[allow(dead_code)]
    fn get_chandelier_stop(&self, direction: PositionDirection) -> Option<Decimal> {
        for config in &self.strategy.indicators {
            if config.indicator_type == IndicatorType::Chandelier {
                let output = match direction {
                    PositionDirection::Long => "exit_long",
                    PositionDirection::Short => "exit_short",
                };
                return self.indicator_engine.get_latest(&config.id, output);
            }
        }
        None
    }

    pub(crate) fn get_atr_value(&self) -> Option<Decimal> {
        for config in &self.strategy.indicators {
            if config.indicator_type == IndicatorType::Atr {
                return self.indicator_engine.get_latest(&config.id, "value");
            }
        }
        None
    }

    fn evaluate_exit_rules(&mut self, candle: &Candle) -> RulesSignal {
        self.evaluate_exit_rules_v2(candle)
    }

    // ============================================================================
    // Pivot Point Methods
    // ============================================================================

    /// Update pivot point tracking based on the current candle
    fn update_pivots(&mut self, candle: &Candle) {
        // Only update if pivot config is enabled
        let period = match &self.pivot_config {
            Some(config) if config.enabled => config.period,
            _ => return,
        };

        // Update tracker and check if we crossed into a new period
        let is_new_period = self.pivot_tracker.update(
            candle.time,
            candle.mid.high,
            candle.mid.low,
            candle.mid.close,
            period,
        );

        // If we crossed into a new period and have previous period data, calculate pivots
        if is_new_period {
            if let Some(pivots) = self.pivot_tracker.calculate_pivots() {
                tracing::info!(
                    "Calculated new {:?} pivots: PP={}, R1={}, S1={}",
                    period,
                    pivots.pp,
                    pivots.r1,
                    pivots.s1
                );
                self.current_pivots = Some(pivots);
            }
        }
    }

    // ============================================================================
    // Shared Helper Methods (used by trigger evaluators)
    // ============================================================================

    /// Resolve a ParameterizedValue to a Decimal using resolved params
    pub(crate) fn resolve_parameterized(&self, value: &ParameterizedValue) -> Option<Decimal> {
        value.resolve(&self.resolved_params)
    }

    /// Get the R:R ratio for a given direction (uses short override if available)
    fn get_rr_ratio(&self, direction: PositionDirection) -> Decimal {
        let risk = &self.strategy.risk_settings;
        let value = match direction {
            PositionDirection::Short => {
                risk.rr_ratio_short.as_ref().unwrap_or(&risk.rr_ratio)
            }
            PositionDirection::Long => &risk.rr_ratio,
        };
        self.resolve_parameterized(value).unwrap_or(dec!(2))
    }

    /// Get the risk method for a given direction (uses short override if available)
    fn get_risk_method(&self, direction: PositionDirection) -> RiskMethod {
        let risk = &self.strategy.risk_settings;
        match direction {
            PositionDirection::Short => {
                risk.risk_method_short.unwrap_or(risk.risk_method)
            }
            PositionDirection::Long => risk.risk_method,
        }
    }

    /// Get the risk value for a given direction (uses short override if available)
    fn get_risk_value(&self, direction: PositionDirection) -> Decimal {
        let risk = &self.strategy.risk_settings;
        let value = match direction {
            PositionDirection::Short => {
                risk.risk_value_short.as_ref().unwrap_or(&risk.risk_value)
            }
            PositionDirection::Long => &risk.risk_value,
        };
        self.resolve_parameterized(value).unwrap_or(dec!(1))
    }

    /// Get the spread buffer pips for a given direction (uses short override if available)
    #[allow(dead_code)]
    fn get_spread_buffer_pips(&self, direction: PositionDirection) -> Decimal {
        let risk = &self.strategy.risk_settings;
        let value = match direction {
            PositionDirection::Short => {
                risk.spread_buffer_pips_short.as_ref().unwrap_or(&risk.spread_buffer_pips)
            }
            PositionDirection::Long => &risk.spread_buffer_pips,
        };
        self.resolve_parameterized(value).unwrap_or(dec!(1))
    }

    pub(crate) fn get_price_value(&self, value: PriceType, candle: &Candle) -> Decimal {
        match value {
            PriceType::Open => candle.mid.open,
            PriceType::High => candle.mid.high,
            PriceType::Low => candle.mid.low,
            PriceType::Close => candle.mid.close,
        }
    }

    /// Reset the engine for a new backtest run
    pub fn reset(&mut self) {
        self.indicator_engine.reset();
        self.position = None;
        self.price_history.clear();
        self.daily_pnl = Decimal::ZERO;
        // Reset pivot tracking
        self.current_pivots = None;
        self.pivot_tracker.reset();
        // Reset regime detector and pattern detection state
        self.regime_detector.reset();
        self.current_candle_index = 0;
        self.patterns_detected = false;
        // Reset HTF state
        for engine in self.htf_indicator_engines.values_mut() {
            engine.reset();
        }
        self.mtf_candle_store.reset();
        for history in self.htf_price_histories.values_mut() {
            history.clear();
        }
        // Re-seed HTF engines with candle[0] from the MTF store.
        // set_mtf_candle_store() feeds the initial candle, but reset() wipes the
        // indicator engines. Since run() calls reset() AFTER set_mtf_candle_store(),
        // we must re-seed here so the first HTF candle isn't lost.
        //
        // Safety: This is safe even on repeated reset() calls — the store indices were
        // just reset to 0 and the engines were just cleared, so re-feeding candle[0]
        // is idempotent. In walk-forward, each window creates a fresh strategy instance
        // with a fresh filter_by_time_range() store, so stale data cannot leak across windows.
        for tf in self.mtf_candle_store.timeframes() {
            if let Some(initial_candle) = self.mtf_candle_store.current_candle(tf) {
                let candle = initial_candle.clone();
                if let Some(engine) = self.htf_indicator_engines.get_mut(tf.as_str()) {
                    engine.on_candle(&candle);
                }
                if let Some(history) = self.htf_price_histories.get_mut(tf.as_str()) {
                    history.push_back(candle);
                }
            }
        }
    }

    /// Check if there's an open position
    pub fn has_position(&self) -> bool {
        self.position.is_some()
    }

    pub fn get_risk_amount(&self, balance: Decimal, direction: PositionDirection) -> Decimal {
        let risk_value = self.get_risk_value(direction);
        let risk_method = self.get_risk_method(direction);

        match risk_method {
            RiskMethod::Percent => {
                let pct = risk_value / dec!(100);
                balance * pct
            }
            RiskMethod::FixedAmount => risk_value,
            RiskMethod::FixedUnits => dec!(100),
        }
    }

    /// Calculate position size based on risk settings, balance, and stop loss distance.
    ///
    /// Formula: position_size = risk_amount / stop_distance
    ///
    /// This ensures consistent risk per trade regardless of stop distance:
    /// - Tight stop = larger position
    /// - Wide stop = smaller position
    /// - Risk amount stays constant
    pub fn calculate_position_size(
        &self,
        balance: Decimal,
        entry_price: Decimal,
        stop_loss: Decimal,
        direction: PositionDirection,
    ) -> Option<Decimal> {
        let risk_value = self.get_risk_value(direction);
        let risk_method = self.get_risk_method(direction);

        // Calculate risk amount based on method
        let risk_amount = match risk_method {
            RiskMethod::Percent => {
                let pct = risk_value / dec!(100);
                balance * pct
            }
            RiskMethod::FixedAmount => risk_value,
            RiskMethod::FixedUnits => {
                // Fixed units means we return the fixed value directly
                return Some(risk_value);
            }
        };

        // Calculate stop distance in price terms
        let stop_distance = (entry_price - stop_loss).abs();

        // Avoid division by zero
        if stop_distance <= Decimal::ZERO {
            return None;
        }

        // Position size = risk_amount / stop_distance
        Some(risk_amount / stop_distance)
    }

    /// Get a snapshot of all current indicator values
    /// Returns HashMap of indicator_id -> (output_name -> value as string)
    pub fn get_indicator_snapshot(&self) -> std::collections::HashMap<String, std::collections::HashMap<String, String>> {
        let mut snapshot = self.indicator_engine.get_snapshot();
        // Include HTF indicator snapshots
        for engine in self.htf_indicator_engines.values() {
            snapshot.extend(engine.get_snapshot());
        }
        snapshot
    }

    /// Advance HTF candle stores and feed new candles to HTF indicator engines.
    /// Called before processing the primary indicator engine on each candle.
    fn advance_htf_engines(&mut self, candle: &Candle) {
        let timeframes: Vec<String> = self.htf_indicator_engines.keys().cloned().collect();
        for tf in timeframes {
            let newly_completed = self.mtf_candle_store.advance(&tf, &candle.time);
            for htf_candle in newly_completed {
                if let Some(engine) = self.htf_indicator_engines.get_mut(&tf) {
                    engine.on_candle(&htf_candle);
                }
                // Update HTF price history
                if let Some(history) = self.htf_price_histories.get_mut(&tf) {
                    history.push_back(htf_candle);
                    if history.len() > self.max_price_history {
                        history.pop_front();
                    }
                }
            }
        }
    }

}

// ============================================================================
// Rules Signal (more detailed than basic Signal)
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum RulesSignal {
    Hold,
    Entry {
        direction: PositionDirection,
        stop_loss: Option<Decimal>,
        take_profit: Option<Decimal>,
        /// ID of the entry rule that triggered this signal
        triggered_rule_id: Option<String>,
        /// Name of the entry rule (if set)
        triggered_rule_name: Option<String>,
        /// Pending order info (if entry should create a pending order instead of market entry)
        pending_order: Option<PendingOrderInfo>,
    },
    Exit {
        reason: String,
        close_percent: f64,
    },
    PartialExit {
        reason: String,
        close_percent: f64,
        new_stop_loss: Option<Decimal>,
    },
}

impl From<RulesSignal> for Signal {
    fn from(signal: RulesSignal) -> Self {
        match signal {
            RulesSignal::Hold => Signal::Hold,
            RulesSignal::Entry { direction, .. } => match direction {
                PositionDirection::Long => Signal::Buy,
                PositionDirection::Short => Signal::Sell,
            },
            RulesSignal::Exit { .. } | RulesSignal::PartialExit { .. } => Signal::ClosePosition,
        }
    }
}

