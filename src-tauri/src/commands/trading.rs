//! Shared trading-domain helpers.
//!
//! AGT-652: the app no longer places or manages orders — execution belongs to
//! the wickd CLI (`wickd approve` / `wickd trade`), the one engine on the
//! machine. The account/positions/orders/place/close Tauri commands were
//! deleted with the account and ticket windows; what remains is the input
//! validation shared by the data/backtest/streaming commands.

/// Valid OANDA instrument format: XXX_YYY where X and Y are currency codes
pub fn is_valid_instrument(instrument: &str) -> bool {
    let parts: Vec<&str> = instrument.split('_').collect();
    if parts.len() != 2 {
        return false;
    }
    parts.iter().all(|p| p.len() == 3 && p.chars().all(|c| c.is_ascii_uppercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_instruments() {
        assert!(is_valid_instrument("EUR_USD"));
        assert!(is_valid_instrument("GBP_JPY"));
    }

    #[test]
    fn rejects_invalid_instruments() {
        assert!(!is_valid_instrument("EURUSD"));
        assert!(!is_valid_instrument("eur_usd"));
        assert!(!is_valid_instrument("EUR_USDX"));
        assert!(!is_valid_instrument("EUR_US1"));
        assert!(!is_valid_instrument("EUR_USD_JPY"));
        assert!(!is_valid_instrument(""));
        assert!(!is_valid_instrument("EUR-USD"));
    }
}
