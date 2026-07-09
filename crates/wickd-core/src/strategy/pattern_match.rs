//! Pattern Match Types
//!
//! Defines pattern match types emitted by the strategy watcher when conditions are met.
//! These represent when a user's strategy rules have been satisfied by market conditions.

use std::collections::HashMap;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::backtest::rules_engine::PositionDirection;

/// Snapshot of indicator values at match time
/// Structure: { indicator_id: { output_name: value_as_string } }
pub type IndicatorSnapshot = HashMap<String, HashMap<String, String>>;

/// Type of pattern match (entry conditions vs exit conditions)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchType {
    /// Entry conditions met
    Entry,
    /// Exit conditions met (full)
    Exit,
    /// Partial exit conditions met
    PartialExit,
}

/// Status of a pattern match
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchStatus {
    /// Pattern matched, awaiting user action
    Pending,
    /// User placed a trade based on this match
    Executed,
    /// User dismissed this match
    Dismissed,
    /// Match expired without action
    Expired,
}

/// A pattern match emitted when strategy conditions are met
///
/// This represents a point in time when the user's defined strategy rules
/// matched current market conditions. The user decides whether to act on it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternMatch {
    /// Unique match ID (UUID)
    pub id: String,
    /// User ID who owns this match
    pub user_id: String,
    /// Strategy config ID that generated this match
    pub config_id: String,
    /// Instrument for this match (e.g., "EUR_USD")
    pub instrument: String,
    /// Type of match (entry/exit conditions)
    pub match_type: MatchType,
    /// Trade direction (for entry matches)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<PositionDirection>,
    /// Current price at match time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_price: Option<Decimal>,
    /// Calculated stop loss level (from strategy rules)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_loss: Option<Decimal>,
    /// Calculated take profit level (from strategy rules)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub take_profit: Option<Decimal>,
    /// Calculated position size (from strategy risk settings)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_size: Option<Decimal>,
    /// Percentage to close (for partial exits)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_percent: Option<f64>,
    /// Name of the strategy rule that produced this match (AGT-624 AC3): a
    /// scripted strategy's signal-map `rule_name`, or a rules-based entry
    /// rule's display name. Optional and additive — absent for signals whose
    /// strategy didn't name the rule, so existing consumers are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_name: Option<String>,
    /// Human-readable description of which conditions matched
    pub reason: String,
    /// Current status of the match
    pub status: MatchStatus,
    /// Snapshot of indicator values at match time (for reference)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indicator_snapshot: Option<IndicatorSnapshot>,
    /// Whether user already has an open position on this instrument
    #[serde(default)]
    pub has_existing_position: bool,
    /// When the user acted on this match (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executed_at: Option<DateTime<Utc>>,
    /// When the match was detected
    pub created_at: DateTime<Utc>,
}

impl PatternMatch {
    /// Create a new entry pattern match
    pub fn entry(
        user_id: String,
        config_id: String,
        instrument: String,
        direction: PositionDirection,
        entry_price: Decimal,
        stop_loss: Option<Decimal>,
        take_profit: Option<Decimal>,
        position_size: Option<Decimal>,
        reason: String,
        indicator_snapshot: Option<IndicatorSnapshot>,
        has_existing_position: bool,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            user_id,
            config_id,
            instrument,
            match_type: MatchType::Entry,
            direction: Some(direction),
            entry_price: Some(entry_price),
            stop_loss,
            take_profit,
            position_size,
            close_percent: None,
            rule_name: None,
            reason,
            status: MatchStatus::Pending,
            indicator_snapshot,
            has_existing_position,
            executed_at: None,
            created_at: Utc::now(),
        }
    }

    /// Create a new exit pattern match
    pub fn exit(
        user_id: String,
        config_id: String,
        instrument: String,
        position_direction: PositionDirection,
        reason: String,
        indicator_snapshot: Option<IndicatorSnapshot>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            user_id,
            config_id,
            instrument,
            match_type: MatchType::Exit,
            direction: Some(position_direction),
            entry_price: None,
            stop_loss: None,
            take_profit: None,
            position_size: None,
            close_percent: Some(100.0),
            rule_name: None,
            reason,
            status: MatchStatus::Pending,
            indicator_snapshot,
            has_existing_position: false,
            executed_at: None,
            created_at: Utc::now(),
        }
    }

    /// Create an exit pattern match with no known broker position (AGT-624
    /// AC3). Scripted strategies track their position virtually inside the
    /// script, so a close signal can fire while the account holds no position
    /// — the direction is the script's business, not the watcher's, hence
    /// `direction: None`. Monitoring surfaces the signal instead of dropping it.
    pub fn exit_unpositioned(
        user_id: String,
        config_id: String,
        instrument: String,
        reason: String,
        indicator_snapshot: Option<IndicatorSnapshot>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            user_id,
            config_id,
            instrument,
            match_type: MatchType::Exit,
            direction: None,
            entry_price: None,
            stop_loss: None,
            take_profit: None,
            position_size: None,
            close_percent: Some(100.0),
            rule_name: None,
            reason,
            status: MatchStatus::Pending,
            indicator_snapshot,
            has_existing_position: false,
            executed_at: None,
            created_at: Utc::now(),
        }
    }

    /// Attach the name of the rule that produced this match (AGT-624 AC3).
    /// Builder-style so the positional constructors stay source-compatible.
    pub fn with_rule_name(mut self, rule_name: Option<String>) -> Self {
        self.rule_name = rule_name;
        self
    }

    /// Create a partial exit pattern match
    pub fn partial_exit(
        user_id: String,
        config_id: String,
        instrument: String,
        position_direction: PositionDirection,
        close_percent: f64,
        reason: String,
        indicator_snapshot: Option<IndicatorSnapshot>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            user_id,
            config_id,
            instrument,
            match_type: MatchType::PartialExit,
            direction: Some(position_direction),
            entry_price: None,
            stop_loss: None,
            take_profit: None,
            position_size: None,
            close_percent: Some(close_percent),
            rule_name: None,
            reason,
            status: MatchStatus::Pending,
            indicator_snapshot,
            has_existing_position: false,
            executed_at: None,
            created_at: Utc::now(),
        }
    }

    /// Mark match as executed (user placed a trade)
    pub fn mark_executed(&mut self) {
        self.status = MatchStatus::Executed;
        self.executed_at = Some(Utc::now());
    }

    /// Mark match as dismissed (user chose not to act)
    pub fn mark_dismissed(&mut self) {
        self.status = MatchStatus::Dismissed;
    }

    /// Mark match as expired
    pub fn mark_expired(&mut self) {
        self.status = MatchStatus::Expired;
    }
}

/// Event payload for pattern match events
#[derive(Debug, Clone, Serialize)]
pub struct PatternMatchEvent {
    pub pattern_match: PatternMatch,
    pub strategy_name: String,
    pub timeframe: String,
}

/// Event payload for strategy status changes
#[derive(Debug, Clone, Serialize)]
pub struct StrategyStatusEvent {
    pub config_id: String,
    pub status: WatcherStatus,
    pub message: Option<String>,
}

/// Status of a strategy watcher
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatcherStatus {
    /// Watcher is running
    Running,
    /// Watcher is stopped
    Stopped,
    /// Watcher encountered an error
    Error,
}

/// Event payload for strategy errors
#[derive(Debug, Clone, Serialize)]
pub struct StrategyErrorEvent {
    pub config_id: String,
    pub error_type: String,
    pub message: String,
}

/// Event payload for match status updates (expiration, etc.)
#[derive(Debug, Clone, Serialize)]
pub struct MatchStatusUpdateEvent {
    pub match_id: String,
    pub config_id: String,
    pub new_status: MatchStatus,
    pub reason: String,
}

/// Debug event for watcher tick (candle processed)
#[derive(Debug, Clone, Serialize)]
pub struct WatcherTickEvent {
    pub config_id: String,
    pub instrument: String,
    pub timeframe: String,
    pub candle_time: String,
    pub close_price: String,
    pub signal_result: String,
}

// Type aliases for backwards compatibility during migration
pub type StrategySignal = PatternMatch;
pub type SignalType = MatchType;
pub type SignalStatus = MatchStatus;
pub type StrategySignalEvent = PatternMatchEvent;
pub type SignalStatusUpdateEvent = MatchStatusUpdateEvent;

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_entry_match_creation() {
        let pattern = PatternMatch::entry(
            "user123".to_string(),
            "config456".to_string(),
            "EUR_USD".to_string(),
            PositionDirection::Long,
            dec!(1.0850),
            Some(dec!(1.0800)),
            Some(dec!(1.0950)),
            Some(dec!(1000)),
            "EMA crossover detected".to_string(),
            None,
            false,
        );

        assert_eq!(pattern.match_type, MatchType::Entry);
        assert_eq!(pattern.direction, Some(PositionDirection::Long));
        assert_eq!(pattern.entry_price, Some(dec!(1.0850)));
        assert_eq!(pattern.stop_loss, Some(dec!(1.0800)));
        assert_eq!(pattern.take_profit, Some(dec!(1.0950)));
        assert_eq!(pattern.status, MatchStatus::Pending);
        assert!(!pattern.has_existing_position);
        assert!(pattern.executed_at.is_none());
    }

    #[test]
    fn test_exit_match_creation() {
        let pattern = PatternMatch::exit(
            "user123".to_string(),
            "config456".to_string(),
            "EUR_USD".to_string(),
            PositionDirection::Long,
            "Exit conditions met".to_string(),
            None,
        );

        assert_eq!(pattern.match_type, MatchType::Exit);
        assert_eq!(pattern.direction, Some(PositionDirection::Long));
        assert_eq!(pattern.close_percent, Some(100.0));
        assert_eq!(pattern.status, MatchStatus::Pending);
    }

    #[test]
    fn test_partial_exit_match_creation() {
        let pattern = PatternMatch::partial_exit(
            "user123".to_string(),
            "config456".to_string(),
            "EUR_USD".to_string(),
            PositionDirection::Short,
            50.0,
            "Partial exit conditions met".to_string(),
            None,
        );

        assert_eq!(pattern.match_type, MatchType::PartialExit);
        assert_eq!(pattern.close_percent, Some(50.0));
    }

    // AGT-624 AC3: a scripted close signal with no broker position surfaces
    // as an Exit with direction unknown instead of being dropped.
    #[test]
    fn test_exit_unpositioned_has_no_direction() {
        let pattern = PatternMatch::exit_unpositioned(
            "user123".to_string(),
            "config456".to_string(),
            "EUR_USD".to_string(),
            "Script exit".to_string(),
            None,
        );

        assert_eq!(pattern.match_type, MatchType::Exit);
        assert_eq!(pattern.direction, None);
        assert_eq!(pattern.close_percent, Some(100.0));
        assert_eq!(pattern.status, MatchStatus::Pending);
        // `direction` is skip_serializing_if none — absent from the NDJSON line.
        let json = serde_json::to_value(&pattern).unwrap();
        assert!(json.get("direction").is_none());
    }

    // AGT-624 AC3: rule_name is additive — absent when unset, present when a
    // strategy names its triggering rule.
    #[test]
    fn test_rule_name_serialization_is_additive() {
        let pattern = PatternMatch::entry(
            "user123".to_string(),
            "config456".to_string(),
            "EUR_USD".to_string(),
            PositionDirection::Long,
            dec!(1.0850),
            Some(dec!(1.0800)),
            Some(dec!(1.0950)),
            None,
            "Test".to_string(),
            None,
            false,
        );
        let json = serde_json::to_value(&pattern).unwrap();
        assert!(json.get("rule_name").is_none(), "unset rule_name must not serialize");

        let named = pattern.with_rule_name(Some("adx_gate".to_string()));
        let json = serde_json::to_value(&named).unwrap();
        assert_eq!(json["rule_name"], "adx_gate");
        // Round-trips, and old payloads without the field still deserialize.
        let back: PatternMatch = serde_json::from_value(json).unwrap();
        assert_eq!(back.rule_name.as_deref(), Some("adx_gate"));
    }

    #[test]
    fn test_match_status_transitions() {
        let mut pattern = PatternMatch::entry(
            "user123".to_string(),
            "config456".to_string(),
            "EUR_USD".to_string(),
            PositionDirection::Long,
            dec!(1.0850),
            Some(dec!(1.0800)),
            Some(dec!(1.0950)),
            None,
            "Test".to_string(),
            None,
            false,
        );

        assert_eq!(pattern.status, MatchStatus::Pending);

        pattern.mark_executed();
        assert_eq!(pattern.status, MatchStatus::Executed);
        assert!(pattern.executed_at.is_some());
    }

    #[test]
    fn test_match_dismissal() {
        let mut pattern = PatternMatch::entry(
            "user123".to_string(),
            "config456".to_string(),
            "EUR_USD".to_string(),
            PositionDirection::Short,
            dec!(1.0850),
            Some(dec!(1.0900)),
            Some(dec!(1.0750)),
            None,
            "Test".to_string(),
            None,
            false,
        );

        pattern.mark_dismissed();
        assert_eq!(pattern.status, MatchStatus::Dismissed);
    }

    #[test]
    fn test_match_expiration() {
        let mut pattern = PatternMatch::exit(
            "user123".to_string(),
            "config456".to_string(),
            "EUR_USD".to_string(),
            PositionDirection::Long,
            "Test".to_string(),
            None,
        );

        pattern.mark_expired();
        assert_eq!(pattern.status, MatchStatus::Expired);
    }
}
