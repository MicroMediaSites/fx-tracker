use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::oanda::types::OandaOrder;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderType {
    Market,
    Limit,
    Stop,
    MarketIfTouched,
    TakeProfit,
    StopLoss,
    TrailingStopLoss,
}

impl From<&str> for OrderType {
    fn from(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "MARKET" => OrderType::Market,
            "LIMIT" => OrderType::Limit,
            "STOP" => OrderType::Stop,
            "MARKET_IF_TOUCHED" => OrderType::MarketIfTouched,
            "TAKE_PROFIT" => OrderType::TakeProfit,
            "STOP_LOSS" => OrderType::StopLoss,
            "TRAILING_STOP_LOSS" => OrderType::TrailingStopLoss,
            _ => OrderType::Market,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderState {
    Pending,
    Filled,
    Triggered,
    Cancelled,
}

impl From<&str> for OrderState {
    fn from(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "PENDING" => OrderState::Pending,
            "FILLED" => OrderState::Filled,
            "TRIGGERED" => OrderState::Triggered,
            "CANCELLED" => OrderState::Cancelled,
            _ => OrderState::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub id: String,
    pub instrument: String,
    pub order_type: OrderType,
    pub units: Decimal,
    pub price: Option<Decimal>,
    pub state: OrderState,
    pub create_time: DateTime<Utc>,
}

impl From<OandaOrder> for Order {
    fn from(oanda: OandaOrder) -> Self {
        let units = oanda.units
            .as_ref()
            .and_then(|u| u.parse().ok())
            .unwrap_or_default();
        let price = oanda.price.as_ref().and_then(|p| p.parse().ok());
        let create_time = DateTime::parse_from_rfc3339(&oanda.create_time)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        Order {
            id: oanda.id,
            instrument: oanda.instrument.unwrap_or_else(|| "N/A".to_string()),
            order_type: OrderType::from(oanda.order_type.as_str()),
            units,
            price,
            state: OrderState::from(oanda.state.as_str()),
            create_time,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn make_limit_order() -> OandaOrder {
        OandaOrder {
            id: "54321".to_string(),
            create_time: "2024-01-15T12:00:00.000000000Z".to_string(),
            order_type: "LIMIT".to_string(),
            instrument: Some("EUR_USD".to_string()),
            units: Some("2000".to_string()),
            state: "PENDING".to_string(),
            price: Some("1.08000".to_string()),
            time_in_force: Some("GTC".to_string()),
            trigger_condition: Some("DEFAULT".to_string()),
            trade_id: None,
        }
    }

    fn make_stop_loss_order() -> OandaOrder {
        OandaOrder {
            id: "54322".to_string(),
            create_time: "2024-01-15T12:30:00.000000000Z".to_string(),
            order_type: "STOP_LOSS".to_string(),
            instrument: None, // Stop loss orders don't always have instrument
            units: None,
            state: "PENDING".to_string(),
            price: Some("1.07500".to_string()),
            time_in_force: Some("GTC".to_string()),
            trigger_condition: Some("DEFAULT".to_string()),
            trade_id: Some("12345".to_string()),
        }
    }

    #[test]
    fn test_order_type_from_string() {
        assert_eq!(OrderType::from("MARKET"), OrderType::Market);
        assert_eq!(OrderType::from("market"), OrderType::Market);
        assert_eq!(OrderType::from("LIMIT"), OrderType::Limit);
        assert_eq!(OrderType::from("STOP"), OrderType::Stop);
        assert_eq!(OrderType::from("MARKET_IF_TOUCHED"), OrderType::MarketIfTouched);
        assert_eq!(OrderType::from("TAKE_PROFIT"), OrderType::TakeProfit);
        assert_eq!(OrderType::from("STOP_LOSS"), OrderType::StopLoss);
        assert_eq!(OrderType::from("TRAILING_STOP_LOSS"), OrderType::TrailingStopLoss);
        assert_eq!(OrderType::from("unknown"), OrderType::Market); // Default
    }

    #[test]
    fn test_order_state_from_string() {
        assert_eq!(OrderState::from("PENDING"), OrderState::Pending);
        assert_eq!(OrderState::from("pending"), OrderState::Pending);
        assert_eq!(OrderState::from("FILLED"), OrderState::Filled);
        assert_eq!(OrderState::from("TRIGGERED"), OrderState::Triggered);
        assert_eq!(OrderState::from("CANCELLED"), OrderState::Cancelled);
        assert_eq!(OrderState::from("unknown"), OrderState::Pending); // Default
    }

    #[test]
    fn test_limit_order_conversion() {
        let order = Order::from(make_limit_order());

        assert_eq!(order.id, "54321");
        assert_eq!(order.instrument, "EUR_USD");
        assert_eq!(order.order_type, OrderType::Limit);
        assert_eq!(order.units, dec!(2000));
        assert_eq!(order.price, Some(dec!(1.08000)));
        assert_eq!(order.state, OrderState::Pending);
    }

    #[test]
    fn test_stop_loss_order_conversion() {
        let order = Order::from(make_stop_loss_order());

        assert_eq!(order.id, "54322");
        assert_eq!(order.instrument, "N/A"); // Fallback when no instrument
        assert_eq!(order.order_type, OrderType::StopLoss);
        assert_eq!(order.units, dec!(0)); // No units for stop loss
        assert_eq!(order.price, Some(dec!(1.07500)));
        assert_eq!(order.state, OrderState::Pending);
    }

    #[test]
    fn test_order_with_no_price() {
        let mut oanda = make_limit_order();
        oanda.price = None;
        let order = Order::from(oanda);
        assert!(order.price.is_none());
    }

    #[test]
    fn test_order_with_invalid_units_defaults_to_zero() {
        let mut oanda = make_limit_order();
        oanda.units = Some("invalid".to_string());
        let order = Order::from(oanda);
        assert_eq!(order.units, dec!(0));
    }

    #[test]
    fn test_order_with_invalid_date_uses_current_time() {
        let mut oanda = make_limit_order();
        oanda.create_time = "not-a-date".to_string();
        let before = Utc::now();
        let order = Order::from(oanda);
        let after = Utc::now();
        assert!(order.create_time >= before && order.create_time <= after);
    }
}
