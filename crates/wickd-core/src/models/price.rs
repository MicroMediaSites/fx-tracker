//! A point-in-time bid/ask for one instrument (OANDA `GET .../pricing`).

use std::str::FromStr;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::oanda::types::StreamPrice;

/// Best bid/ask for an instrument at `time`.
#[derive(Debug, Clone, PartialEq)]
pub struct InstrumentPrice {
    pub instrument: String,
    pub bid: Decimal,
    pub ask: Decimal,
    pub time: DateTime<Utc>,
}

impl TryFrom<StreamPrice> for InstrumentPrice {
    type Error = String;

    /// Fails when a side has no price bucket or a field doesn't parse (e.g.
    /// a halted instrument), so callers can skip it.
    fn try_from(p: StreamPrice) -> Result<Self, String> {
        let side = |b: &[crate::oanda::types::PriceBucket], name: &str| {
            b.first()
                .ok_or_else(|| format!("{}: no {name}", p.instrument))
                .and_then(|x| Decimal::from_str(&x.price).map_err(|e| e.to_string()))
        };
        Ok(Self {
            bid: side(&p.bids, "bid")?,
            ask: side(&p.asks, "ask")?,
            time: p.time.parse().map_err(|e: chrono::ParseError| e.to_string())?,
            instrument: p.instrument,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oanda::types::PricingResponse;
    use rust_decimal_macros::dec;

    #[test]
    fn pricing_response_converts_and_skips_empty_sides() {
        let r: PricingResponse = serde_json::from_str(
            r#"{"prices":[{"instrument":"EUR_USD","time":"2026-11-09T03:00:00Z","tradeable":true,
            "bids":[{"price":"1.11540","liquidity":1000000}],"asks":[{"price":"1.11548","liquidity":1000000}]},
            {"instrument":"XYZ_ABC","time":"2026-11-09T03:00:00Z","tradeable":false,"bids":[],"asks":[]}]}"#,
        )
        .unwrap();
        let prices: Vec<InstrumentPrice> = r
            .prices
            .into_iter()
            .filter_map(|p| InstrumentPrice::try_from(p).ok())
            .collect();
        assert_eq!(prices.len(), 1);
        assert_eq!(prices[0].instrument, "EUR_USD");
        assert_eq!((prices[0].bid, prices[0].ask), (dec!(1.11540), dec!(1.11548)));
    }
}
