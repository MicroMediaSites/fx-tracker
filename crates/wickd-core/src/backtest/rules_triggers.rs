//! V2 Trigger evaluation methods for RulesEngine
//!
//! Separated from rules_engine.rs for better organization.
//! Contains all V2-specific trigger chain, condition, and rule evaluation logic.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal_macros::dec;

use crate::models::Candle;
use super::rules_types::*;
use super::rules_engine::{RulesEngine, RulesSignal};
use super::strategy::PendingOrderInfo;

// ============================================================================
// V2 Entry Rule Evaluation
// ============================================================================

impl RulesEngine {
    /// Resolve a PendingOrderConfig into a PendingOrderInfo by evaluating the DataSource price.
    /// Returns None if the order type is Market (treat as market order) or if price cannot be resolved.
    fn resolve_pending_order(&self, rule: &EntryRule, candle: &Candle) -> Option<PendingOrderInfo> {
        let config = rule.pending_order.as_ref()?;

        // Market orders don't create pending orders
        if config.order_type == EntryOrderType::Market {
            return None;
        }

        // Resolve the DataSource price to a concrete Decimal
        let price = self.resolve_data_source_v2(&config.price, candle, 0)?;

        Some(PendingOrderInfo {
            order_type: config.order_type,
            price,
            expiry_bars: config.expiry_bars,
        })
    }

    pub(crate) fn evaluate_entry_rules_v2(&self, candle: &Candle) -> RulesSignal {
        for rule in &self.strategy.entry_rules {
            let applies_to_long = rule.direction.applies_to(PositionDirection::Long);
            let applies_to_short = rule.direction.applies_to(PositionDirection::Short);

            let triggered = self.evaluate_entry_rule_v2(rule, candle);

            if triggered {
                let pending_order = self.resolve_pending_order(rule, candle);

                if applies_to_long {
                    // If SL is on wrong side of entry, skip this trade entirely.
                    // Do NOT fall through to subsequent rules — the first triggered
                    // rule has priority, and if its SL is invalid the trade is skipped.
                    return match self.calculate_sl_tp_for_signal(PositionDirection::Long, candle) {
                        Some((stop_loss, take_profit)) => RulesSignal::Entry {
                            direction: PositionDirection::Long,
                            stop_loss: Some(stop_loss),
                            take_profit: Some(take_profit),
                            triggered_rule_id: Some(rule.id.clone()),
                            triggered_rule_name: rule.name.clone(),
                            pending_order,
                        },
                        None => RulesSignal::Hold,
                    };
                } else if applies_to_short {
                    return match self.calculate_sl_tp_for_signal(PositionDirection::Short, candle) {
                        Some((stop_loss, take_profit)) => RulesSignal::Entry {
                            direction: PositionDirection::Short,
                            stop_loss: Some(stop_loss),
                            take_profit: Some(take_profit),
                            triggered_rule_id: Some(rule.id.clone()),
                            triggered_rule_name: rule.name.clone(),
                            pending_order,
                        },
                        None => RulesSignal::Hold,
                    };
                }
            }
        }

        RulesSignal::Hold
    }

    pub(crate) fn evaluate_entry_rules_v2_with_position(&mut self, candle: &Candle) -> RulesSignal {
        // Use index-based iteration to avoid holding an immutable borrow on
        // self.strategy.entry_rules while calling &mut self methods below.
        // Note: evaluate_entry_rule_v2 must remain &self — if changed to &mut self,
        // this index pattern will fail to compile.
        for i in 0..self.strategy.entry_rules.len() {
            let applies_to_long = self.strategy.entry_rules[i].direction.applies_to(PositionDirection::Long);
            let applies_to_short = self.strategy.entry_rules[i].direction.applies_to(PositionDirection::Short);

            let triggered = self.evaluate_entry_rule_v2(&self.strategy.entry_rules[i], candle);

            if triggered {
                // Clone rule data before mutable borrow in open_position
                let rule_id = self.strategy.entry_rules[i].id.clone();
                let rule_name = self.strategy.entry_rules[i].name.clone();
                let pending_order = self.resolve_pending_order(&self.strategy.entry_rules[i], candle);

                if applies_to_long {
                    self.open_position(PositionDirection::Long, candle);
                    // First triggered rule has priority. If open_position skipped
                    // due to wrong-side SL, return Hold — do NOT fall through to
                    // subsequent rules, preserving pre-fix rule priority semantics.
                    return if self.position.is_some() {
                        RulesSignal::Entry {
                            direction: PositionDirection::Long,
                            stop_loss: self.position.as_ref().map(|p| p.stop_loss),
                            take_profit: self.position.as_ref().map(|p| p.take_profit),
                            triggered_rule_id: Some(rule_id),
                            triggered_rule_name: rule_name,
                            pending_order,
                        }
                    } else {
                        RulesSignal::Hold
                    };
                } else if applies_to_short {
                    self.open_position(PositionDirection::Short, candle);
                    return if self.position.is_some() {
                        RulesSignal::Entry {
                            direction: PositionDirection::Short,
                            stop_loss: self.position.as_ref().map(|p| p.stop_loss),
                            take_profit: self.position.as_ref().map(|p| p.take_profit),
                            triggered_rule_id: Some(rule_id),
                            triggered_rule_name: rule_name,
                            pending_order,
                        }
                    } else {
                        RulesSignal::Hold
                    };
                }
            }
        }

        RulesSignal::Hold
    }

    /// Evaluate an entry rule V2 - supports both new conditions and legacy trigger_chain
    pub fn evaluate_entry_rule_v2(&self, rule: &EntryRule, candle: &Candle) -> bool {
        // Prefer new conditions format
        if !rule.conditions.is_empty() {
            return self.evaluate_conditions(&rule.conditions, candle);
        }
        // Fall back to legacy trigger_chain
        if let Some(ref chain) = rule.trigger_chain {
            return self.evaluate_trigger_chain(chain, candle);
        }
        false
    }
}

// ============================================================================
// V2 Exit Rule Evaluation
// ============================================================================

impl RulesEngine {
    /// V2 Exit rule evaluation - trigger chain based
    pub(crate) fn evaluate_exit_rules_v2(&mut self, candle: &Candle) -> RulesSignal {
        let pos = match &self.position {
            Some(p) => p.clone(),
            None => return RulesSignal::Hold,
        };

        // Sort exit rules by priority (higher first)
        let mut sorted_rules: Vec<ExitRule> = self.strategy.exit_rules.to_vec();
        sorted_rules.sort_by(|a, b| b.priority.cmp(&a.priority));

        for rule in &sorted_rules {
            if pos.triggered_partials.contains(&rule.id) {
                continue;
            }

            let applies_to_position = rule.direction.applies_to(pos.direction);
            if !applies_to_position {
                continue;
            }

            let triggered = self.evaluate_exit_rule_v2(rule, candle, &pos);

            if triggered {
                // Resolve close_percent (defaults to 100% if not specified or invalid)
                let close_percent = self.resolve_parameterized(&rule.close_percent)
                    .and_then(|d| d.to_f64())
                    .unwrap_or(100.0);

                if close_percent >= 100.0 || close_percent >= pos.remaining_percent {
                    self.position = None;
                    return RulesSignal::Exit {
                        reason: rule.name.clone().unwrap_or_else(|| rule.id.clone()),
                        close_percent: 100.0,
                    };
                } else {
                    if let Some(ref mut p) = self.position {
                        p.remaining_percent -= close_percent;
                        p.triggered_partials.push(rule.id.clone());
                    }

                    return RulesSignal::PartialExit {
                        reason: rule.name.clone().unwrap_or_else(|| rule.id.clone()),
                        close_percent,
                        new_stop_loss: self.position.as_ref().map(|p| p.stop_loss),
                    };
                }
            }
        }

        RulesSignal::Hold
    }

    /// Evaluate a V2 trigger chain for exit rules (with position context)
    /// DEPRECATED: Use evaluate_exit_condition instead
    fn evaluate_exit_trigger_chain_v2(&self, chain: &TriggerChain, candle: &Candle, pos: &PositionState) -> bool {
        let mut result = self.evaluate_trigger_v2_with_position(&chain.primary, candle, pos);

        for chained in &chain.chain {
            let chained_result = self.evaluate_trigger_v2_with_position(&chained.trigger, candle, pos);
            match chained.operator {
                ChainOperator::And => result = result && chained_result,
                ChainOperator::Or => result = result || chained_result,
            }
        }

        result
    }

    /// Evaluate a single exit condition (with NOT support and position context)
    ///
    /// Uses grouped evaluation where:
    /// - Groups are determined by splitting at AND operators
    /// - Triggers within a group are OR'd together (any must pass)
    /// - Groups are AND'd together (all groups must pass)
    ///
    /// Example: A OR B AND C = (A OR B) AND (C)
    fn evaluate_exit_condition(&self, condition: &Condition, candle: &Candle, pos: &PositionState) -> bool {
        // Skip disabled conditions (treat as passing)
        if self.is_condition_disabled(condition) {
            return true;
        }

        // Build groups: split chain at AND operators
        // - Groups are AND'd together (all groups must pass)
        // - Triggers within a group are OR'd (any trigger in group must pass)
        let mut groups: Vec<Vec<(&Trigger, bool)>> = vec![vec![(
            &condition.primary.trigger,
            condition.primary.negated,
        )]];

        for chained in &condition.chain {
            match chained.operator {
                ChainOperator::Or => {
                    // Add to current group (OR'd together)
                    groups.last_mut().unwrap().push((
                        &chained.trigger.trigger,
                        chained.trigger.negated,
                    ));
                }
                ChainOperator::And => {
                    // Start new group (AND'd with previous groups)
                    groups.push(vec![(
                        &chained.trigger.trigger,
                        chained.trigger.negated,
                    )]);
                }
            }
        }

        // Evaluate: ALL groups must pass (AND'd together)
        // Within each group: ANY trigger must pass (OR'd together)
        groups.iter().all(|group| {
            group.iter().any(|(trigger, negated)| {
                let result = self.evaluate_trigger_v2_with_position(trigger, candle, pos);
                if *negated { !result } else { result }
            })
        })
    }

    /// Evaluate all exit conditions (AND'd together, with position context)
    fn evaluate_exit_conditions(&self, conditions: &[Condition], candle: &Candle, pos: &PositionState) -> bool {
        if conditions.is_empty() {
            return false;
        }
        conditions.iter().all(|c| self.evaluate_exit_condition(c, candle, pos))
    }

    /// Evaluate an exit rule V2 - supports both new conditions and legacy trigger_chain
    fn evaluate_exit_rule_v2(&self, rule: &ExitRule, candle: &Candle, pos: &PositionState) -> bool {
        if !rule.conditions.is_empty() {
            return self.evaluate_exit_conditions(&rule.conditions, candle, pos);
        }
        if let Some(ref chain) = rule.trigger_chain {
            return self.evaluate_exit_trigger_chain_v2(chain, candle, pos);
        }
        false
    }

    /// Evaluate a V2 trigger with position context (for exit rules)
    fn evaluate_trigger_v2_with_position(&self, trigger: &Trigger, candle: &Candle, pos: &PositionState) -> bool {
        match trigger {
            Trigger::Givens(t) => self.evaluate_givens_trigger(t, candle),
            Trigger::Cross(t) => self.evaluate_cross_v2(t, candle),
            Trigger::Compare(t) => self.evaluate_compare_v2(t, candle),
            Trigger::Threshold(t) => self.evaluate_threshold_v2(t, candle),
            Trigger::RiskReward(t) => self.evaluate_risk_reward_v2(t, candle, pos),
            Trigger::PercentOfTp(t) => self.evaluate_percent_of_tp_v2(t, candle, pos),
            Trigger::Time(t) => self.evaluate_time_v2(t, pos, candle),
            Trigger::TimeInRange(t) => self.evaluate_time_in_range(t, candle),
            Trigger::DayOfWeek(t) => self.evaluate_day_of_week(t, candle),
        }
    }
}

// ============================================================================
// V2 Exit Trigger Helpers
// ============================================================================

impl RulesEngine {
    /// Evaluate V2 risk/reward trigger
    fn evaluate_risk_reward_v2(&self, trigger: &RiskRewardTrigger, candle: &Candle, pos: &PositionState) -> bool {
        let ratio = self.resolve_parameterized(&trigger.ratio).unwrap_or(dec!(2));
        let risk = (pos.entry_price - pos.stop_loss).abs();
        let current_profit = match pos.direction {
            PositionDirection::Long => candle.mid.close - pos.entry_price,
            PositionDirection::Short => pos.entry_price - candle.mid.close,
        };
        current_profit >= risk * ratio
    }

    /// Evaluate V2 percent of take profit trigger
    fn evaluate_percent_of_tp_v2(&self, trigger: &PercentOfTpTrigger, candle: &Candle, pos: &PositionState) -> bool {
        let percent = self.resolve_parameterized(&trigger.percent).unwrap_or(dec!(50));
        let total_distance = (pos.take_profit - pos.entry_price).abs();
        let current_progress = match pos.direction {
            PositionDirection::Long => candle.mid.close - pos.entry_price,
            PositionDirection::Short => pos.entry_price - candle.mid.close,
        };
        if total_distance == dec!(0) {
            return false;
        }
        let progress_percent = (current_progress / total_distance) * dec!(100);
        progress_percent >= percent
    }

    /// Evaluate V2 time-based trigger
    fn evaluate_time_v2(&self, trigger: &TimeTrigger, pos: &PositionState, candle: &Candle) -> bool {
        use shared::TimeCondition;
        let value = self.resolve_parameterized(&trigger.value).unwrap_or(dec!(0));
        match trigger.condition {
            TimeCondition::BarCount => Decimal::from(pos.bars_since_entry as i64) >= value,
            TimeCondition::Minutes => {
                // Calculate actual minutes elapsed since entry
                let elapsed = candle.time.signed_duration_since(pos.entry_time);
                let minutes_elapsed = elapsed.num_minutes();
                Decimal::from(minutes_elapsed) >= value
            }
            TimeCondition::Hours => {
                // Calculate actual hours elapsed since entry
                let elapsed = candle.time.signed_duration_since(pos.entry_time);
                let hours_elapsed = elapsed.num_hours();
                Decimal::from(hours_elapsed) >= value
            }
        }
    }

    /// Evaluate time-in-range session filter trigger
    fn evaluate_time_in_range(&self, trigger: &TimeInRangeTrigger, candle: &Candle) -> bool {
        use chrono::Timelike;
        // Validate ranges — reject invalid values rather than producing nonsensical results
        if trigger.start_hour >= 24 || trigger.end_hour >= 24
            || trigger.start_minute >= 60 || trigger.end_minute >= 60
        {
            tracing::warn!(
                "TimeInRange trigger has invalid values: start={}:{}, end={}:{}",
                trigger.start_hour, trigger.start_minute, trigger.end_hour, trigger.end_minute
            );
            return false;
        }
        let candle_hour = candle.time.hour() as u8;
        let candle_minute = candle.time.minute() as u8;
        let candle_minutes = candle_hour as u16 * 60 + candle_minute as u16;
        let start_minutes = trigger.start_hour as u16 * 60 + trigger.start_minute as u16;
        let end_minutes = trigger.end_hour as u16 * 60 + trigger.end_minute as u16;

        if start_minutes <= end_minutes {
            // Normal range (e.g., 08:00-16:00)
            candle_minutes >= start_minutes && candle_minutes < end_minutes
        } else {
            // Wraps midnight (e.g., 22:00-02:00)
            candle_minutes >= start_minutes || candle_minutes < end_minutes
        }
    }

    /// Evaluate day-of-week session filter trigger
    fn evaluate_day_of_week(&self, trigger: &DayOfWeekTrigger, candle: &Candle) -> bool {
        use chrono::Datelike;
        // Validate: filter out invalid day values (must be 0-6)
        if trigger.days.iter().any(|&d| d > 6) {
            tracing::warn!(
                "DayOfWeek trigger has invalid day values: {:?} (must be 0-6)",
                trigger.days
            );
            return false;
        }
        // chrono: num_days_from_monday() returns 0=Mon..6=Sun
        // Our API: 0=Sun, 1=Mon, ..., 6=Sat
        let chrono_day = candle.time.weekday().num_days_from_monday();
        let our_day = match chrono_day {
            6 => 0u8,  // Sun
            d => (d + 1) as u8,  // Mon=1, Tue=2, ..., Sat=6
        };
        let day_in_list = trigger.days.contains(&our_day);
        if trigger.exclude { !day_in_list } else { day_in_list }
    }
}

// ============================================================================
// V2 Trigger Chain & Condition Evaluation
// ============================================================================

impl RulesEngine {
    /// Evaluate a V2 trigger chain (primary trigger + AND/OR chained triggers)
    /// DEPRECATED: Use evaluate_condition instead
    pub fn evaluate_trigger_chain(&self, chain: &TriggerChain, candle: &Candle) -> bool {
        let mut result = self.evaluate_trigger_v2(&chain.primary, candle);

        for chained in &chain.chain {
            let chained_result = self.evaluate_trigger_v2(&chained.trigger, candle);

            match chained.operator {
                ChainOperator::And => {
                    result = result && chained_result;
                }
                ChainOperator::Or => {
                    result = result || chained_result;
                }
            }
        }

        result
    }

    /// Check if a condition is disabled (should be skipped)
    fn is_condition_disabled(&self, condition: &Condition) -> bool {
        if let Some(ref disabled) = condition.disabled {
            match disabled {
                shared::ParameterizedValue::Fixed(v) => *v != 0.0,
                shared::ParameterizedValue::Reference(r) => {
                    self.resolved_params.get(&r.param_id)
                        .map(|v| *v != 0.0)
                        .unwrap_or(false)
                }
            }
        } else {
            false
        }
    }

    /// Evaluate a single condition (with NOT support on each trigger)
    ///
    /// Uses grouped evaluation where:
    /// - Groups are determined by splitting at AND operators
    /// - Triggers within a group are OR'd together (any must pass)
    /// - Groups are AND'd together (all groups must pass)
    ///
    /// Example: A OR B AND C = (A OR B) AND (C)
    pub fn evaluate_condition(&self, condition: &Condition, candle: &Candle) -> bool {
        // Skip disabled conditions (treat as passing)
        if self.is_condition_disabled(condition) {
            tracing::debug!("[DISABLED] Skipping disabled condition: {:?}", condition.disabled);
            return true;
        }

        // Build groups: split chain at AND operators
        // - Groups are AND'd together (all groups must pass)
        // - Triggers within a group are OR'd (any trigger in group must pass)
        // Example: A OR B AND C AND D OR E = (A OR B) AND (C) AND (D OR E)
        let mut groups: Vec<Vec<(&Trigger, bool)>> = vec![vec![(
            &condition.primary.trigger,
            condition.primary.negated,
        )]];

        for chained in &condition.chain {
            match chained.operator {
                ChainOperator::Or => {
                    // Add to current group (OR'd together)
                    groups.last_mut().unwrap().push((
                        &chained.trigger.trigger,
                        chained.trigger.negated,
                    ));
                }
                ChainOperator::And => {
                    // Start new group (AND'd with previous groups)
                    groups.push(vec![(
                        &chained.trigger.trigger,
                        chained.trigger.negated,
                    )]);
                }
            }
        }

        // Evaluate: ALL groups must pass (AND'd together)
        // Within each group: ANY trigger must pass (OR'd together)
        groups.iter().all(|group| {
            group.iter().any(|(trigger, negated)| {
                let result = self.evaluate_trigger_v2(trigger, candle);
                if *negated { !result } else { result }
            })
        })
    }

    /// Evaluate all conditions (AND'd together)
    pub fn evaluate_conditions(&self, conditions: &[Condition], candle: &Candle) -> bool {
        if conditions.is_empty() {
            return false;
        }
        conditions.iter().all(|c| self.evaluate_condition(c, candle))
    }

    /// Evaluate a single V2 trigger
    pub fn evaluate_trigger_v2(&self, trigger: &Trigger, candle: &Candle) -> bool {
        match trigger {
            Trigger::Givens(t) => self.evaluate_givens_trigger(t, candle),
            Trigger::Cross(t) => self.evaluate_cross_v2(t, candle),
            Trigger::Compare(t) => self.evaluate_compare_v2(t, candle),
            Trigger::Threshold(t) => self.evaluate_threshold_v2(t, candle),
            Trigger::TimeInRange(t) => self.evaluate_time_in_range(t, candle),
            Trigger::DayOfWeek(t) => self.evaluate_day_of_week(t, candle),
            Trigger::RiskReward(_) | Trigger::PercentOfTp(_) | Trigger::Time(_) => {
                // Exit triggers that need position context - handled separately
                false
            }
        }
    }
}

// ============================================================================
// V2 Individual Trigger Evaluators
// ============================================================================

impl RulesEngine {
    /// Evaluate a V2 threshold trigger
    fn evaluate_threshold_v2(&self, trigger: &ThresholdTrigger, candle: &Candle) -> bool {
        let source_val = self.resolve_data_source_v2(&trigger.source, candle, 0);
        let threshold = self.resolve_parameterized(&trigger.value);

        match (source_val, threshold) {
            (Some(val), Some(thresh)) => {
                match trigger.operator.as_str() {
                    "above" | ">" => val > thresh,
                    "below" | "<" => val < thresh,
                    ">=" | "gte" => val >= thresh,
                    "<=" | "lte" => val <= thresh,
                    "crosses_above" => {
                        // Current value > threshold AND previous value <= threshold
                        if let Some(prev_val) = self.resolve_data_source_v2(&trigger.source, candle, 1) {
                            val > thresh && prev_val <= thresh
                        } else {
                            false
                        }
                    }
                    "crosses_below" => {
                        // Current value < threshold AND previous value >= threshold
                        if let Some(prev_val) = self.resolve_data_source_v2(&trigger.source, candle, 1) {
                            val < thresh && prev_val >= thresh
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Check whether ADX is configured (present in strategy indicators), regardless
    /// of whether it has produced a value yet. Used to distinguish "not configured"
    /// (ADX is optional, treat as pass) from "warming up" (ADX is required, block until ready).
    fn is_adx_configured(&self) -> bool {
        self.strategy.indicators.iter().any(|c| c.indicator_type == IndicatorType::Adx)
    }

    /// Shared trending evaluation for both TrendingUp and TrendingDown.
    ///
    /// `is_up`: true for TrendingUp (price > SMA20 > SMA50), false for TrendingDown.
    fn evaluate_trending(
        &self,
        candle: &Candle,
        adx: Option<Decimal>,
        sma20: Option<Decimal>,
        sma50: Option<Decimal>,
        is_up: bool,
    ) -> bool {
        let price = candle.mid.close;
        let sma_aligned = match (sma20, sma50) {
            (Some(s20), Some(s50)) => {
                if is_up { price > s20 && s20 > s50 } else { price < s20 && s20 < s50 }
            }
            _ => false,
        };

        // Distinguish "ADX not in strategy" from "ADX configured but still warming up"
        let adx_configured = self.is_adx_configured();
        let adx_ok = match adx {
            Some(adx_val) => adx_val > dec!(20),
            None if adx_configured => false, // Configured but warming up — block
            None => true,                    // Not configured — don't require it
        };

        let result = if sma20.is_some() && sma50.is_some() {
            sma_aligned && adx_ok
        } else if adx.is_some() {
            adx_ok
        } else {
            false
        };

        let direction = if is_up { "trending_up" } else { "trending_down" };
        tracing::trace!(
            "givens::{} — adx={:?} (configured={}) sma20={:?} sma50={:?} price={} sma_aligned={} adx_ok={} → {}",
            direction, adx, adx_configured, sma20, sma50, price, sma_aligned, adx_ok, result
        );
        result
    }

    /// Evaluate a givens (market regime) trigger
    ///
    /// Trending detection uses a tiered approach:
    /// - ADX configured + value present: require ADX > 20 AND SMA alignment
    /// - ADX configured but still warming up (None): block (conservative, no spurious early trades)
    /// - ADX not configured: SMA alignment alone is sufficient
    /// - No SMA or ADX indicators: false
    ///
    /// ADX threshold is 20 (changed from 25 in PR #252). The original 25 is calibrated
    /// for daily charts — on H1/H4 timeframes, ADX(14) covers only ~14 hours and values
    /// are inherently lower. 20 indicates a developing trend, which is sufficient for
    /// entry signal filtering on intraday data.
    fn evaluate_givens_trigger(&self, trigger: &GivensTrigger, candle: &Candle) -> bool {
        let adx = self.get_indicator_value_by_type(IndicatorType::Adx, "value");
        let sma20 = self.get_sma_value(20);
        let sma50 = self.get_sma_value(50);
        let atr = self.get_indicator_value_by_type(IndicatorType::Atr, "value");
        let bb_upper = self.get_indicator_value_by_type(IndicatorType::Bollinger, "upper");
        let bb_lower = self.get_indicator_value_by_type(IndicatorType::Bollinger, "lower");
        let bb_middle = self.get_indicator_value_by_type(IndicatorType::Bollinger, "middle");

        match trigger.regime {
            MarketRegime::TrendingUp => self.evaluate_trending(candle, adx, sma20, sma50, true),
            MarketRegime::TrendingDown => self.evaluate_trending(candle, adx, sma20, sma50, false),
            MarketRegime::Ranging => {
                match (adx, bb_upper, bb_lower, bb_middle) {
                    (Some(adx_val), Some(upper), Some(lower), Some(middle)) => {
                        if adx_val >= dec!(20) || middle == Decimal::ZERO {
                            return false;
                        }
                        let bb_width = (upper - lower) / middle;
                        bb_width < dec!(0.02)
                    }
                    (Some(adx_val), _, _, _) => adx_val < dec!(20),
                    _ => false,
                }
            }
            MarketRegime::SrTested => {
                let price = candle.mid.close;
                let pip_value = dec!(0.0001);
                let distance_threshold = dec!(20) * pip_value;

                self.sr_zones.iter().any(|zone| {
                    (price - zone.upper_price).abs() <= distance_threshold
                        || (price - zone.lower_price).abs() <= distance_threshold
                })
            }
            MarketRegime::HighVolatility => {
                match atr {
                    Some(atr_val) => atr_val > dec!(0.0075),
                    None => false,
                }
            }
            MarketRegime::LowVolatility => {
                match atr {
                    Some(atr_val) => atr_val < dec!(0.0025),
                    None => false,
                }
            }
            MarketRegime::AtBullishGap
            | MarketRegime::AtBearishGap
            | MarketRegime::AtDemandZone
            | MarketRegime::AtSupplyZone
            | MarketRegime::AtBullishOb
            | MarketRegime::AtBearishOb
            | MarketRegime::RetestingSupport
            | MarketRegime::RetestingResistance => {
                // Price action patterns - delegate to RegimeDetector
                // Note: patterns must be detected first via prepare_for_backtest()
                self.regime_detector.is_regime_active(
                    trigger.regime.clone(),
                    candle,
                    adx,
                    sma20,
                    sma50,
                    atr,
                    bb_upper,
                    bb_lower,
                    bb_middle,
                )
            }
            // Trading Sessions (UTC)
            MarketRegime::LondonSession => {
                use chrono::Timelike;
                let hour = candle.time.hour() as u8;
                hour >= 8 && hour < 17
            }
            MarketRegime::UsSession => {
                use chrono::Timelike;
                let hour = candle.time.hour() as u8;
                hour >= 13 && hour < 22
            }
            MarketRegime::AsianSession => {
                use chrono::Timelike;
                let hour = candle.time.hour() as u8;
                hour < 9  // 00:00-09:00 UTC
            }
            // Divergence (uses config fields from GivensTrigger)
            MarketRegime::Divergence => {
                self.evaluate_divergence_from_givens(trigger, candle)
            }
        }
    }

    /// Evaluate divergence from givens trigger config
    fn evaluate_divergence_from_givens(&self, trigger: &GivensTrigger, _candle: &Candle) -> bool {
        // Extract config from trigger, with defaults
        let divergence_type = match &trigger.divergence_type {
            Some(dt) => dt.clone(),
            None => return false,  // No divergence type configured
        };
        let indicator = match &trigger.divergence_indicator {
            Some(ind) => ind.clone(),
            None => return false,  // No indicator configured
        };
        let output = trigger.divergence_output.as_deref().unwrap_or("value");
        let lookback = trigger.divergence_lookback.unwrap_or(50) as usize;
        let strength = trigger.divergence_swing_strength.unwrap_or(5) as usize;

        // Need enough price history for swing detection
        if self.price_history.len() < lookback {
            return false;
        }

        // Find swing points in price history
        let (swing_highs, swing_lows) = self.find_swing_points(lookback, strength);

        match divergence_type {
            shared::DivergenceType::Bullish | shared::DivergenceType::HiddenBullish => {
                if swing_lows.len() < 2 {
                    return false;
                }
                let (recent_low, recent_bars) = swing_lows[swing_lows.len() - 1];
                let (prev_low, prev_bars) = swing_lows[swing_lows.len() - 2];

                let recent_ind = self.indicator_engine.get_output(&indicator, output, recent_bars);
                let prev_ind = self.indicator_engine.get_output(&indicator, output, prev_bars);

                match (recent_ind, prev_ind) {
                    (Some(recent_ind_val), Some(prev_ind_val)) => {
                        match divergence_type {
                            shared::DivergenceType::Bullish => recent_low < prev_low && recent_ind_val > prev_ind_val,
                            shared::DivergenceType::HiddenBullish => recent_low > prev_low && recent_ind_val < prev_ind_val,
                            _ => false,
                        }
                    }
                    _ => false,
                }
            }
            shared::DivergenceType::Bearish | shared::DivergenceType::HiddenBearish => {
                if swing_highs.len() < 2 {
                    return false;
                }
                let (recent_high, recent_bars) = swing_highs[swing_highs.len() - 1];
                let (prev_high, prev_bars) = swing_highs[swing_highs.len() - 2];

                let recent_ind = self.indicator_engine.get_output(&indicator, output, recent_bars);
                let prev_ind = self.indicator_engine.get_output(&indicator, output, prev_bars);

                match (recent_ind, prev_ind) {
                    (Some(recent_ind_val), Some(prev_ind_val)) => {
                        match divergence_type {
                            shared::DivergenceType::Bearish => recent_high > prev_high && recent_ind_val < prev_ind_val,
                            shared::DivergenceType::HiddenBearish => recent_high < prev_high && recent_ind_val > prev_ind_val,
                            _ => false,
                        }
                    }
                    _ => false,
                }
            }
        }
    }

    pub(crate) fn get_indicator_value_by_type(&self, indicator_type: IndicatorType, output: &str) -> Option<Decimal> {
        for config in &self.strategy.indicators {
            if config.indicator_type == indicator_type {
                if let Some(val) = self.indicator_engine.get_latest(&config.id, output) {
                    return Some(val);
                }
            }
        }
        None
    }

    fn get_sma_value(&self, period: u32) -> Option<Decimal> {
        for config in &self.strategy.indicators {
            if config.indicator_type == IndicatorType::Sma {
                if let Some(p) = config.params.get("period") {
                    if let Some(param_period) = p.as_fixed() {
                        if param_period as u32 == period {
                            return self.indicator_engine.get_latest(&config.id, "value");
                        }
                    }
                }
            }
        }
        None
    }

    /// Evaluate a V2 cross trigger
    fn evaluate_cross_v2(&self, trigger: &CrossTrigger, candle: &Candle) -> bool {
        let lookback = self.resolve_parameterized(&trigger.lookback)
            .map(|d| d.to_u32().unwrap_or(1).max(1) as usize)
            .unwrap_or(1);

        for i in 0..lookback {
            let current_left = self.resolve_data_source_v2(&trigger.left, candle, i);
            let current_right = self.resolve_data_source_v2(&trigger.right, candle, i);
            let prev_left = self.resolve_data_source_v2(&trigger.left, candle, i + 1);
            let prev_right = self.resolve_data_source_v2(&trigger.right, candle, i + 1);

            let crossed = match (current_left, current_right, prev_left, prev_right) {
                (Some(cl), Some(cr), Some(pl), Some(pr)) => {
                    match trigger.direction.as_str() {
                        "above" => pl <= pr && cl > cr,
                        "below" => pl >= pr && cl < cr,
                        _ => false,
                    }
                }
                _ => false,
            };

            if crossed {
                return true;
            }
        }

        false
    }

    /// Evaluate a V2 compare trigger (including is_within)
    fn evaluate_compare_v2(&self, trigger: &CompareTrigger, candle: &Candle) -> bool {
        let lookback = self.resolve_parameterized(&trigger.lookback)
            .map(|d| d.to_u32().unwrap_or(1).max(1) as usize)
            .unwrap_or(1);

        for i in 0..lookback {
            let left = self.resolve_data_source_v2(&trigger.left, candle, i);
            let right = self.resolve_data_source_v2(&trigger.right, candle, i);

            let matched = match (left, right) {
                (Some(l), Some(r)) => {
                    match trigger.operator.as_str() {
                        ">" => l > r,
                        "<" => l < r,
                        ">=" => l >= r,
                        "<=" => l <= r,
                        "==" => l == r,
                        "!=" => l != r,
                        "is_within" => {
                            if let Some(ref distance) = trigger.distance {
                                self.evaluate_is_within(l, r, distance, candle)
                            } else {
                                false
                            }
                        }
                        // Legacy operators
                        "above" => l > r,
                        "below" => l < r,
                        "equals" => l == r,
                        "gte" => l >= r,
                        "lte" => l <= r,
                        _ => false,
                    }
                }
                _ => false,
            };

            if matched {
                return true;
            }
        }

        false
    }

    /// Evaluate "is_within" operator - checks if left is within distance of right
    fn evaluate_is_within(&self, left: Decimal, right: Decimal, distance: &DistanceConfig, _candle: &Candle) -> bool {
        let distance_value = match self.resolve_parameterized(&distance.value) {
            Some(v) => v,
            None => return false,
        };

        let threshold = match distance.unit {
            DistanceUnit::Pips => {
                let pip_value = dec!(0.0001);
                distance_value * pip_value
            }
            DistanceUnit::Atr => {
                if let Some(atr) = self.get_atr_value() {
                    distance_value * atr
                } else {
                    return false;
                }
            }
            DistanceUnit::Percent => {
                (distance_value / dec!(100)) * right.abs()
            }
        };

        (left - right).abs() <= threshold
    }

    /// Find swing high and low points in price history
    /// Returns (swing_highs, swing_lows) where each is Vec<(price, bars_ago)>
    fn find_swing_points(&self, lookback: usize, strength: usize) -> (Vec<(Decimal, usize)>, Vec<(Decimal, usize)>) {
        let mut swing_highs: Vec<(Decimal, usize)> = Vec::new();
        let mut swing_lows: Vec<(Decimal, usize)> = Vec::new();

        let history_len = self.price_history.len();
        if history_len < strength * 2 + 1 {
            return (swing_highs, swing_lows);
        }

        // Only look back 'lookback' bars
        let start_idx = history_len.saturating_sub(lookback);

        // Check each bar (except the most recent 'strength' bars, which can't be confirmed yet)
        for center_idx in (start_idx + strength)..(history_len.saturating_sub(strength)) {
            let center_candle = &self.price_history[center_idx];
            let center_high = center_candle.mid.high;
            let center_low = center_candle.mid.low;

            // Check swing high
            let mut is_swing_high = true;
            for i in (center_idx.saturating_sub(strength))..center_idx {
                if self.price_history[i].mid.high >= center_high {
                    is_swing_high = false;
                    break;
                }
            }
            if is_swing_high {
                for i in (center_idx + 1)..=(center_idx + strength).min(history_len - 1) {
                    if self.price_history[i].mid.high >= center_high {
                        is_swing_high = false;
                        break;
                    }
                }
            }
            if is_swing_high {
                let bars_ago = history_len - 1 - center_idx;
                swing_highs.push((center_high, bars_ago));
            }

            // Check swing low
            let mut is_swing_low = true;
            for i in (center_idx.saturating_sub(strength))..center_idx {
                if self.price_history[i].mid.low <= center_low {
                    is_swing_low = false;
                    break;
                }
            }
            if is_swing_low {
                for i in (center_idx + 1)..=(center_idx + strength).min(history_len - 1) {
                    if self.price_history[i].mid.low <= center_low {
                        is_swing_low = false;
                        break;
                    }
                }
            }
            if is_swing_low {
                let bars_ago = history_len - 1 - center_idx;
                swing_lows.push((center_low, bars_ago));
            }
        }

        (swing_highs, swing_lows)
    }
}

// ============================================================================
// V2 Data Source Resolution
// ============================================================================

impl RulesEngine {
    /// Resolve a V2 data source to a Decimal value
    pub(crate) fn resolve_data_source_v2(&self, source: &DataSource, candle: &Candle, offset: usize) -> Option<Decimal> {
        match source {
            DataSource::Indicator(src) => {
                if src.capture == CaptureMode::AtEntry {
                    if let Some(ref pos) = self.position {
                        let key = format!("indicator.{}.{}", src.indicator, src.output);
                        if let Some(captured) = pos.captured_values.get(&key) {
                            return Some(captured.current_value);
                        }
                    }
                }
                let total_offset = src.offset + offset;
                // Route to HTF indicator engine if timeframe is explicitly specified
                if let Some(ref tf) = src.timeframe {
                    if let Some(htf_engine) = self.htf_indicator_engines.get(tf) {
                        return htf_engine.get_output(&src.indicator, &src.output, total_offset);
                    }
                }
                // Try primary engine first.
                if let Some(val) = self.indicator_engine.get_output(&src.indicator, &src.output, total_offset) {
                    return Some(val);
                }
                // Fall back to HTF engines. This handles the case where an indicator has
                // timeframe set in its config but the trigger data source doesn't repeat
                // it — the indicator lives in the HTF engine, not the primary one.
                //
                // Why not propagate timeframe at deserialization? The AI builds strategies
                // where indicator configs have timeframe but triggers reference indicators
                // by name only. Populating src.timeframe from indicator config would require
                // a cross-referencing pass during strategy validation that doesn't exist yet.
                // This fallback is the pragmatic fix until that validation layer is added.
                //
                // Sort keys for deterministic iteration order across runs.
                let mut htf_keys: Vec<&String> = self.htf_indicator_engines.keys().collect();
                htf_keys.sort();
                let mut found_in: Vec<&str> = Vec::new();
                let mut result: Option<Decimal> = None;
                for key in &htf_keys {
                    if let Some(htf_engine) = self.htf_indicator_engines.get(*key) {
                        if let Some(val) = htf_engine.get_output(&src.indicator, &src.output, total_offset) {
                            if result.is_none() {
                                result = Some(val);
                            }
                            found_in.push(key);
                        }
                    }
                }
                if found_in.len() > 1 {
                    tracing::warn!(
                        "Ambiguous HTF indicator '{}': found in {} engines ({:?}). \
                         Using first match ({}). Add explicit timeframe to the data source to resolve.",
                        src.indicator, found_in.len(), found_in, found_in[0]
                    );
                }
                if result.is_none() {
                    tracing::warn!(
                        "Indicator '{}' output '{}' not found in primary or any HTF engine — \
                         check indicator name for typos",
                        src.indicator, src.output
                    );
                }
                result
            }
            DataSource::Price(src) => {
                if src.capture == CaptureMode::AtEntry {
                    if let Some(ref pos) = self.position {
                        let key = format!("price.{}", src.value);
                        if let Some(captured) = pos.captured_values.get(&key) {
                            return Some(captured.current_value);
                        }
                    }
                }
                // Route to HTF price history if timeframe is specified
                if let Some(ref tf) = src.timeframe {
                    if let Some(htf_history) = self.htf_price_histories.get(tf) {
                        if !htf_history.is_empty() {
                            let htf_candle = if offset > 0 && htf_history.len() > offset {
                                &htf_history[htf_history.len() - 1 - offset]
                            } else {
                                htf_history.back().unwrap()
                            };
                            return Some(self.get_price_value(src.value, htf_candle));
                        }
                        return None;
                    }
                }
                let candle_to_use = if offset > 0 && self.price_history.len() > offset {
                    &self.price_history[self.price_history.len() - 1 - offset]
                } else {
                    candle
                };
                Some(self.get_price_value(src.value, candle_to_use))
            }
            DataSource::Fixed(src) => {
                Decimal::try_from(src.fixed).ok()
            }
            DataSource::Parameter(src) => {
                self.resolved_params.get(&src.param_id)
                    .and_then(|v| Decimal::try_from(*v).ok())
            }
            DataSource::SRZone(src) => {
                let zone = if let Some(ref zone_id) = src.zone_id {
                    self.sr_zones.iter().find(|z| &z.id == zone_id)
                } else {
                    let price = candle.mid.close;
                    self.sr_zones.iter().min_by_key(|z| {
                        let upper_dist = (price - z.upper_price).abs();
                        let lower_dist = (price - z.lower_price).abs();
                        if upper_dist < lower_dist {
                            upper_dist
                        } else {
                            lower_dist
                        }
                    })
                };

                zone.map(|z| match src.target {
                    SRTarget::Upper => z.upper_price,
                    SRTarget::Lower => z.lower_price,
                    SRTarget::Midpoint => (z.upper_price + z.lower_price) / dec!(2),
                })
            }
            DataSource::Pivot(src) => {
                self.current_pivots.as_ref().map(|p| p.get_level(src.level))
            }
            DataSource::Variable(src) => {
                // Find the variable definition
                let var_def = self.strategy.variables
                    .iter()
                    .find(|v| v.id == src.variable)?;

                // Combine source offset with lookback offset
                let total_offset = src.offset + offset;
                self.resolve_variable_expression(&var_def.expression, candle, total_offset)
            }
            DataSource::Numeric(v) => {
                Decimal::try_from(*v).ok()
            }
            DataSource::Pattern(p) => {
                let needed = super::patterns::candles_needed(&p.pattern);
                let total_offset = match offset.checked_add(p.offset) {
                    Some(v) => v,
                    None => return None,
                };
                let required = match needed.checked_add(total_offset) {
                    Some(v) => v,
                    None => return None,
                };
                let history_len = self.price_history.len();
                if history_len < required {
                    return None;
                }
                let start = history_len - required;
                let end = history_len - total_offset;
                let candle_slice = &self.price_history[start..end];
                if super::patterns::detect_pattern(&p.pattern, candle_slice) {
                    Some(Decimal::ONE)
                } else {
                    Some(Decimal::ZERO)
                }
            }
        }
    }

    /// Resolve a variable expression to a Decimal value
    pub(crate) fn resolve_variable_expression(&self, expr: &VariableExpression, candle: &Candle, offset: usize) -> Option<Decimal> {
        match expr {
            VariableExpression::Distance { left, right, absolute } => {
                let left_val = self.resolve_data_source_v2(left, candle, offset)?;
                let right_val = self.resolve_data_source_v2(right, candle, offset)?;
                let diff = left_val - right_val;
                Some(if *absolute { diff.abs() } else { diff })
            }
            VariableExpression::Ratio { numerator, denominator } => {
                let num = self.resolve_data_source_v2(numerator, candle, offset)?;
                let denom = self.resolve_data_source_v2(denominator, candle, offset)?;
                if denom.is_zero() {
                    None
                } else {
                    Some(num / denom)
                }
            }
            VariableExpression::Change { source, bars } => {
                // Change = past value - current value
                // Positive = source was higher in the past (declining)
                // Negative = source was lower in the past (rising)
                let past_val = self.resolve_data_source_v2(source, candle, offset + bars)?;
                let current_val = self.resolve_data_source_v2(source, candle, offset)?;
                Some(past_val - current_val)
            }
            VariableExpression::Value { source, operations } => {
                // Start with the base source value
                let mut result = self.resolve_data_source_v2(source, candle, offset)?;

                // Apply operations left-to-right (no operator precedence)
                if let Some(ops) = operations {
                    for op in ops {
                        let operand = self.resolve_data_source_v2(&op.operand, candle, offset)?;
                        result = match op.operator {
                            MathOperator::Add => result + operand,
                            MathOperator::Subtract => result - operand,
                            MathOperator::Multiply => result * operand,
                            MathOperator::Divide => {
                                if operand.is_zero() {
                                    return None; // Division by zero
                                }
                                result / operand
                            }
                            MathOperator::Pow => {
                                // Integer exponents only — avoids f64 precision loss
                                let exp = match operand.to_i64() {
                                    Some(e) => e,
                                    None => return None,
                                };
                                // Guard: i64::MIN can't be negated; cap exponent magnitude
                                let abs_exp = match exp.checked_abs() {
                                    Some(a) if a <= 100 => a as u32,
                                    _ => return None,
                                };
                                let mut acc = Decimal::ONE;
                                for _ in 0..abs_exp {
                                    acc *= result;
                                }
                                if exp < 0 {
                                    if acc.is_zero() { return None; }
                                    Decimal::ONE / acc
                                } else {
                                    acc
                                }
                            }
                            MathOperator::Mod => {
                                if operand.is_zero() {
                                    return None;
                                }
                                result % operand
                            }
                        };
                    }
                }
                Some(result)
            }
            VariableExpression::Abs { source } => {
                let val = self.resolve_data_source_v2(source, candle, offset)?;
                Some(val.abs())
            }
            VariableExpression::Negate { source } => {
                let val = self.resolve_data_source_v2(source, candle, offset)?;
                Some(-val)
            }
            VariableExpression::Min { left, right } => {
                let l = self.resolve_data_source_v2(left, candle, offset)?;
                let r = self.resolve_data_source_v2(right, candle, offset)?;
                Some(std::cmp::min(l, r))
            }
            VariableExpression::Max { left, right } => {
                let l = self.resolve_data_source_v2(left, candle, offset)?;
                let r = self.resolve_data_source_v2(right, candle, offset)?;
                Some(std::cmp::max(l, r))
            }
            VariableExpression::Highest { source, period } => {
                let n = period.resolve(&self.resolved_params)?.to_usize()?;
                if n == 0 { return None; }
                // Require all N values — return None if any bar is missing
                let mut highest = self.resolve_data_source_v2(source, candle, offset)?;
                for i in 1..n {
                    let val = self.resolve_data_source_v2(source, candle, offset + i)?;
                    highest = std::cmp::max(highest, val);
                }
                Some(highest)
            }
            VariableExpression::Lowest { source, period } => {
                let n = period.resolve(&self.resolved_params)?.to_usize()?;
                if n == 0 { return None; }
                let mut lowest = self.resolve_data_source_v2(source, candle, offset)?;
                for i in 1..n {
                    let val = self.resolve_data_source_v2(source, candle, offset + i)?;
                    lowest = std::cmp::min(lowest, val);
                }
                Some(lowest)
            }
            VariableExpression::Sum { source, period } => {
                let n = period.resolve(&self.resolved_params)?.to_usize()?;
                if n == 0 { return None; }
                let mut sum = self.resolve_data_source_v2(source, candle, offset)?;
                for i in 1..n {
                    sum += self.resolve_data_source_v2(source, candle, offset + i)?;
                }
                Some(sum)
            }
            VariableExpression::Average { source, period } => {
                let n = period.resolve(&self.resolved_params)?.to_usize()?;
                if n == 0 { return None; }
                let mut sum = self.resolve_data_source_v2(source, candle, offset)?;
                for i in 1..n {
                    sum += self.resolve_data_source_v2(source, candle, offset + i)?;
                }
                Some(sum / Decimal::from(n))
            }
            VariableExpression::Conditional { condition_left, operator, condition_right, true_value, false_value } => {
                let cl = self.resolve_data_source_v2(condition_left, candle, offset)?;
                let cr = self.resolve_data_source_v2(condition_right, candle, offset)?;
                let condition_met = match operator {
                    ComparisonOperator::GreaterThan => cl > cr,
                    ComparisonOperator::GreaterThanOrEqual => cl >= cr,
                    ComparisonOperator::LessThan => cl < cr,
                    ComparisonOperator::LessThanOrEqual => cl <= cr,
                    ComparisonOperator::Equal => cl == cr,
                    ComparisonOperator::IsWithin => {
                        tracing::warn!("is_within operator not supported in conditional expressions");
                        return None;
                    }
                };
                if condition_met {
                    self.resolve_data_source_v2(true_value, candle, offset)
                } else {
                    self.resolve_data_source_v2(false_value, candle, offset)
                }
            }
        }
    }
}
