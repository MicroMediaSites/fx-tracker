use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::oanda::types::OandaPosition;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub instrument: String,
    pub units: Decimal,
    pub average_price: Decimal,
    pub unrealized_pl: Decimal,
    pub realized_pl: Decimal,
}

impl Position {
    pub fn flat(instrument: &str) -> Self {
        Self {
            instrument: instrument.to_string(),
            units: Decimal::ZERO,
            average_price: Decimal::ZERO,
            unrealized_pl: Decimal::ZERO,
            realized_pl: Decimal::ZERO,
        }
    }

    pub fn is_flat(&self) -> bool {
        self.units == Decimal::ZERO
    }

    pub fn is_long(&self) -> bool {
        self.units > Decimal::ZERO
    }

    pub fn is_short(&self) -> bool {
        self.units < Decimal::ZERO
    }
}

impl From<OandaPosition> for Position {
    fn from(oanda: OandaPosition) -> Self {
        let long_units: Decimal = oanda.long.units.parse().unwrap_or_default();
        let short_units: Decimal = oanda.short.units.parse().unwrap_or_default();
        let units = long_units - short_units.abs();

        let average_price = if units > Decimal::ZERO {
            oanda.long.average_price.as_ref().and_then(|p| p.parse().ok()).unwrap_or_default()
        } else if units < Decimal::ZERO {
            oanda.short.average_price.as_ref().and_then(|p| p.parse().ok()).unwrap_or_default()
        } else {
            Decimal::ZERO
        };

        let unrealized_pl = oanda.unrealized_pl.parse().unwrap_or_default();
        let realized_pl = oanda.realized_pl.parse().unwrap_or_default();

        Position {
            instrument: oanda.instrument,
            units,
            average_price,
            unrealized_pl,
            realized_pl,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oanda::types::OandaPositionSide;
    use rust_decimal_macros::dec;

    fn make_long_position() -> OandaPosition {
        OandaPosition {
            instrument: "EUR_USD".to_string(),
            realized_pl: "150.2500".to_string(),
            unrealized_pl: "25.0000".to_string(),
            long: OandaPositionSide {
                units: "5000".to_string(),
                average_price: Some("1.08500".to_string()),
                realized_pl: Some("150.2500".to_string()),
                unrealized_pl: Some("25.0000".to_string()),
            },
            short: OandaPositionSide {
                units: "0".to_string(),
                average_price: None,
                realized_pl: None,
                unrealized_pl: None,
            },
        }
    }

    fn make_short_position() -> OandaPosition {
        OandaPosition {
            instrument: "GBP_USD".to_string(),
            realized_pl: "-50.0000".to_string(),
            unrealized_pl: "-10.0000".to_string(),
            long: OandaPositionSide {
                units: "0".to_string(),
                average_price: None,
                realized_pl: None,
                unrealized_pl: None,
            },
            short: OandaPositionSide {
                units: "3000".to_string(),
                average_price: Some("1.26000".to_string()),
                realized_pl: Some("-50.0000".to_string()),
                unrealized_pl: Some("-10.0000".to_string()),
            },
        }
    }

    fn make_flat_position() -> OandaPosition {
        OandaPosition {
            instrument: "USD_JPY".to_string(),
            realized_pl: "100.0000".to_string(),
            unrealized_pl: "0.0000".to_string(),
            long: OandaPositionSide {
                units: "0".to_string(),
                average_price: None,
                realized_pl: Some("100.0000".to_string()),
                unrealized_pl: None,
            },
            short: OandaPositionSide {
                units: "0".to_string(),
                average_price: None,
                realized_pl: None,
                unrealized_pl: None,
            },
        }
    }

    #[test]
    fn test_long_position_conversion() {
        let pos = Position::from(make_long_position());

        assert_eq!(pos.instrument, "EUR_USD");
        assert_eq!(pos.units, dec!(5000));
        assert_eq!(pos.average_price, dec!(1.08500));
        assert_eq!(pos.unrealized_pl, dec!(25.0000));
        assert_eq!(pos.realized_pl, dec!(150.2500));
    }

    #[test]
    fn test_short_position_conversion() {
        let pos = Position::from(make_short_position());

        assert_eq!(pos.instrument, "GBP_USD");
        assert_eq!(pos.units, dec!(-3000)); // Negative for short
        assert_eq!(pos.average_price, dec!(1.26000));
        assert_eq!(pos.unrealized_pl, dec!(-10.0000));
        assert_eq!(pos.realized_pl, dec!(-50.0000));
    }

    #[test]
    fn test_flat_position_conversion() {
        let pos = Position::from(make_flat_position());

        assert_eq!(pos.instrument, "USD_JPY");
        assert_eq!(pos.units, dec!(0));
        assert_eq!(pos.average_price, dec!(0)); // No price for flat positions
        assert_eq!(pos.unrealized_pl, dec!(0));
        assert_eq!(pos.realized_pl, dec!(100.0000));
    }

    #[test]
    fn test_position_is_long() {
        let long_pos = Position::from(make_long_position());
        assert!(long_pos.is_long());
        assert!(!long_pos.is_short());
        assert!(!long_pos.is_flat());
    }

    #[test]
    fn test_position_is_short() {
        let short_pos = Position::from(make_short_position());
        assert!(short_pos.is_short());
        assert!(!short_pos.is_long());
        assert!(!short_pos.is_flat());
    }

    #[test]
    fn test_position_is_flat() {
        let flat_pos = Position::from(make_flat_position());
        assert!(flat_pos.is_flat());
        assert!(!flat_pos.is_long());
        assert!(!flat_pos.is_short());
    }

    #[test]
    fn test_flat_constructor() {
        let flat = Position::flat("EUR_USD");
        assert_eq!(flat.instrument, "EUR_USD");
        assert!(flat.is_flat());
        assert_eq!(flat.units, dec!(0));
        assert_eq!(flat.average_price, dec!(0));
        assert_eq!(flat.unrealized_pl, dec!(0));
        assert_eq!(flat.realized_pl, dec!(0));
    }

    #[test]
    fn test_position_with_invalid_units_defaults_to_zero() {
        let mut oanda = make_long_position();
        oanda.long.units = "invalid".to_string();
        let pos = Position::from(oanda);
        assert_eq!(pos.units, dec!(0));
    }
}
