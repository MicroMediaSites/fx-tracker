// Golden script (STRATEGY_ABI.md "Surprise feed" example, ABI v4).
//
// The H-015-live shape from the edge-hunt registry: a big economic surprise
// (|z| > 1.5 actual-vs-forecast, z-scored against pre-committed discovery
// stats) blows through the book; the overshoot is a liquidity vacuum, not a
// price-discovery level, so this fades RSI extremes ONLY in the window right
// after a big high-impact surprise on the instrument's currency legs.
//
// Sentinels: surprise_z() returns -9999 when there is no scored release
// (no calendar dropped in ~/.wickd/calendar/, no release at or before this
// candle, or the latest release has a forecast but no published actual yet
// — such a release is skipped until its actual is backfilled).
// surprise_hours_ago() returns -1 in the same cases and always refers to the
// SAME release as surprise_z(), so gating a z on its recency is sound.
//
// Honesty caveat (see STRATEGY_ABI.md): backtest spread cost is flat, but
// real spreads blow out around news — event-window results are optimistic.
// @indicators: [
//   { "id": "rsi", "type": "rsi", "params": { "period": 14 } }
// ]
// @parameters: [
//   { "id": "z_threshold", "name": "Surprise |z| threshold", "type": "number", "default": 1.5, "min": 0.0, "max": 5.0 },
//   { "id": "window_hours", "name": "Post-release window (hours)", "type": "number", "default": 24.0, "min": 1.0, "max": 96.0 },
//   { "id": "rsi_oversold", "name": "RSI Oversold", "type": "number", "default": 30.0 },
//   { "id": "rsi_overbought", "name": "RSI Overbought", "type": "number", "default": 70.0 }
// ]

fn on_candle() {
    // Warmup guard: indicator() returns 0 before it is populated, and an
    // RSI of 0 would read as deeply oversold.
    if bar_count() < 15 { return "hold"; }

    let z = surprise_z(); // most recent scored high-impact leg release
    if z <= -9999.0 { return "hold"; } // sentinel: no scored release yet
    let zt = param("z_threshold");
    if z < zt && z > 0.0 - zt { return "hold"; } // only BIG surprises

    let hrs = surprise_hours_ago(); // same release as the z above
    if hrs < 0.0 || hrs > param("window_hours") { return "hold"; }

    let rsi_val = indicator("rsi", "value");
    if rsi_val < param("rsi_oversold") {
        return #{ signal: "buy", rule_name: "big-surprise fade (long)" };
    }
    if rsi_val > param("rsi_overbought") {
        return #{ signal: "sell", rule_name: "big-surprise fade (short)" };
    }
    "hold"
}
