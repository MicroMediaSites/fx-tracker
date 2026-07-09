// Golden script (STRATEGY_ABI.md example: "Minimal script").
//
// The smallest valid on_candle() contract: no @indicators/@parameters metadata,
// no risk fields — just a signal returned as a plain string ("buy" / "sell" /
// "hold" / "close").
fn on_candle() {
    if is_bullish() {
        "buy"
    } else if is_bearish() {
        "sell"
    } else {
        "hold"
    }
}
