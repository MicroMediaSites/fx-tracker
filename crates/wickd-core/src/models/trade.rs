use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::oanda::types::OandaTrade;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeState {
    Open,
    Closed,
    CloseWhenTradeable,
}

impl From<&str> for TradeState {
    fn from(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "OPEN" => TradeState::Open,
            "CLOSED" => TradeState::Closed,
            "CLOSE_WHEN_TRADEABLE" => TradeState::CloseWhenTradeable,
            _ => TradeState::Open, // Default fallback
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub id: String,
    pub instrument: String,
    pub open_price: Decimal,
    pub open_time: DateTime<Utc>,
    pub units: Decimal,
    pub realized_pl: Decimal,
    pub unrealized_pl: Option<Decimal>,
    pub state: TradeState,
    pub close_time: Option<DateTime<Utc>>,
    pub close_price: Option<Decimal>,
    /// Strategy that placed the trade (AGT-630 / AGT-631): the `tag` OANDA
    /// echoes back from the placing order's `clientExtensions`. `None` for
    /// manual or pre-attribution trades — those report as unattributed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
}

impl Trade {
    pub fn is_long(&self) -> bool {
        self.units > Decimal::ZERO
    }

    pub fn is_short(&self) -> bool {
        self.units < Decimal::ZERO
    }

    pub fn is_open(&self) -> bool {
        self.state == TradeState::Open
    }

    pub fn total_pl(&self) -> Decimal {
        self.realized_pl + self.unrealized_pl.unwrap_or(Decimal::ZERO)
    }
}

impl From<OandaTrade> for Trade {
    fn from(oanda: OandaTrade) -> Self {
        let open_price = oanda.price.parse().unwrap_or_default();
        // Use initial_units for direction/size - current_units is 0 for closed trades!
        // This ensures we preserve the original trade direction (long/short) after closing.
        let units: Decimal = oanda.initial_units.parse().unwrap_or_default();
        let realized_pl: Decimal = oanda.realized_pl.parse().unwrap_or_default();
        let unrealized_pl = oanda.unrealized_pl
            .as_ref()
            .and_then(|s| s.parse().ok());

        let open_time = DateTime::parse_from_rfc3339(&oanda.open_time)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        let close_time = oanda.close_time
            .as_ref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let close_price: Option<Decimal> = oanda.average_close_price
            .as_ref()
            .and_then(|s| s.parse().ok());

        // Debug logging for trade conversion - helps diagnose entry/exit/P&L discrepancies
        let direction = if units > Decimal::ZERO { "Long" } else { "Short" };
        tracing::debug!(
            "[TradeConvert] {} {} | id: {} | initial_units: {} | current_units: {} | oanda.price: {} → open_price: {} | oanda.average_close_price: {:?} → close_price: {:?} | realized_pl: {}",
            oanda.instrument, direction, oanda.id,
            oanda.initial_units, oanda.current_units,
            oanda.price, open_price,
            oanda.average_close_price, close_price,
            realized_pl
        );

        Trade {
            id: oanda.id,
            instrument: oanda.instrument,
            open_price,
            open_time,
            units,
            realized_pl,
            unrealized_pl,
            state: TradeState::from(oanda.state.as_str()),
            close_time,
            close_price,
            // OANDA echoes the placing order's clientExtensions onto the trade;
            // the `tag` is the strategy name (AGT-630). `None` when unattributed.
            strategy: oanda.client_extensions.and_then(|c| c.tag),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn make_open_trade() -> OandaTrade {
        OandaTrade {
            id: "12345".to_string(),
            instrument: "EUR_USD".to_string(),
            price: "1.08500".to_string(),
            open_time: "2024-01-15T10:30:00.000000000Z".to_string(),
            initial_units: "1000".to_string(),
            current_units: "1000".to_string(),
            realized_pl: "0.0000".to_string(),
            unrealized_pl: Some("25.5000".to_string()),
            state: "OPEN".to_string(),
            financing: Some("-0.5000".to_string()),
            close_time: None,
            average_close_price: None,
            initial_margin_required: Some("33.33".to_string()),
            closing_transaction_ids: vec![],
            dividend_adjustment: None,
            client_extensions: None,
        }
    }

    fn make_closed_trade() -> OandaTrade {
        OandaTrade {
            id: "12346".to_string(),
            instrument: "GBP_USD".to_string(),
            price: "1.26000".to_string(),
            open_time: "2024-01-14T09:00:00.000000000Z".to_string(),
            initial_units: "-500".to_string(),
            current_units: "0".to_string(),
            realized_pl: "-15.2500".to_string(),
            unrealized_pl: None,
            state: "CLOSED".to_string(),
            financing: Some("-1.2500".to_string()),
            close_time: Some("2024-01-15T14:00:00.000000000Z".to_string()),
            average_close_price: Some("1.26305".to_string()),
            initial_margin_required: None,
            closing_transaction_ids: vec!["98765".to_string()],
            dividend_adjustment: None,
            client_extensions: None,
        }
    }

    #[test]
    fn test_trade_state_from_string() {
        assert_eq!(TradeState::from("OPEN"), TradeState::Open);
        assert_eq!(TradeState::from("open"), TradeState::Open);
        assert_eq!(TradeState::from("CLOSED"), TradeState::Closed);
        assert_eq!(TradeState::from("closed"), TradeState::Closed);
        assert_eq!(TradeState::from("CLOSE_WHEN_TRADEABLE"), TradeState::CloseWhenTradeable);
        assert_eq!(TradeState::from("unknown"), TradeState::Open); // Default
    }

    #[test]
    fn test_open_trade_conversion() {
        let oanda_trade = make_open_trade();
        let trade = Trade::from(oanda_trade);

        assert_eq!(trade.id, "12345");
        assert_eq!(trade.instrument, "EUR_USD");
        assert_eq!(trade.open_price, dec!(1.08500));
        assert_eq!(trade.units, dec!(1000));
        assert_eq!(trade.realized_pl, dec!(0.0000));
        assert_eq!(trade.unrealized_pl, Some(dec!(25.5000)));
        assert_eq!(trade.state, TradeState::Open);
        assert!(trade.close_time.is_none());
        assert!(trade.close_price.is_none());
    }

    #[test]
    fn test_closed_trade_conversion() {
        let oanda_trade = make_closed_trade();
        let trade = Trade::from(oanda_trade);

        assert_eq!(trade.id, "12346");
        assert_eq!(trade.instrument, "GBP_USD");
        assert_eq!(trade.open_price, dec!(1.26000));
        // Use initial_units (-500) not current_units (0) to preserve direction
        assert_eq!(trade.units, dec!(-500)); // Negative = short position
        assert!(trade.is_short()); // Should correctly identify as short
        assert_eq!(trade.realized_pl, dec!(-15.2500));
        assert!(trade.unrealized_pl.is_none());
        assert_eq!(trade.state, TradeState::Closed);
        assert!(trade.close_time.is_some());
        assert_eq!(trade.close_price, Some(dec!(1.26305)));
    }

    #[test]
    fn test_trade_is_long() {
        let mut oanda_trade = make_open_trade();
        oanda_trade.initial_units = "1000".to_string();
        let trade = Trade::from(oanda_trade);
        assert!(trade.is_long());
        assert!(!trade.is_short());
    }

    #[test]
    fn test_trade_is_short() {
        let mut oanda_trade = make_open_trade();
        oanda_trade.initial_units = "-500".to_string();
        let trade = Trade::from(oanda_trade);
        assert!(trade.is_short());
        assert!(!trade.is_long());
    }

    #[test]
    fn test_trade_is_open() {
        let open_trade = Trade::from(make_open_trade());
        assert!(open_trade.is_open());

        let closed_trade = Trade::from(make_closed_trade());
        assert!(!closed_trade.is_open());
    }

    #[test]
    fn test_trade_total_pl() {
        let trade = Trade::from(make_open_trade());
        assert_eq!(trade.total_pl(), dec!(25.5000));

        let closed_trade = Trade::from(make_closed_trade());
        assert_eq!(closed_trade.total_pl(), dec!(-15.2500));
    }

    #[test]
    fn test_closed_trade_carries_strategy_from_client_extensions() {
        // AGT-631: OANDA echoes the placing order's clientExtensions onto the
        // trade, so a closed trade names the strategy that placed it (AGT-630).
        let mut oanda_trade = make_closed_trade();
        oanda_trade.client_extensions = Some(crate::oanda::types::ClientExtensions {
            tag: Some("h004-surprise".to_string()),
            comment: Some("wickd strategy=h004-surprise".to_string()),
        });
        let trade = Trade::from(oanda_trade);
        assert_eq!(trade.strategy.as_deref(), Some("h004-surprise"));
    }

    #[test]
    fn test_trade_without_client_extensions_is_unattributed() {
        // A manual or pre-attribution trade carries no clientExtensions → None.
        let trade = Trade::from(make_closed_trade());
        assert_eq!(trade.strategy, None);
    }

    #[test]
    fn test_trade_with_invalid_price_defaults_to_zero() {
        let mut oanda_trade = make_open_trade();
        oanda_trade.price = "invalid".to_string();
        let trade = Trade::from(oanda_trade);
        assert_eq!(trade.open_price, dec!(0));
    }

    #[test]
    fn test_trade_with_invalid_date_uses_current_time() {
        let mut oanda_trade = make_open_trade();
        oanda_trade.open_time = "not-a-date".to_string();
        let before = Utc::now();
        let trade = Trade::from(oanda_trade);
        let after = Utc::now();
        assert!(trade.open_time >= before && trade.open_time <= after);
    }
}
