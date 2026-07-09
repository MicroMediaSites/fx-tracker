// Golden script (STRATEGY_ABI.md "candle clock" example, ABI v2).
//
// A session-gated strategy: only trades during the Asian session
// (00:00–07:59 UTC), using candle_hour() rather than bar-index arithmetic —
// bar counting is unsound across weekend/missing-candle gaps, the candle
// clock is not. candle_time() (Unix seconds of the candle's open) covers
// gates that need real elapsed time rather than an hour-of-day.
// @indicators: [
//   { "id": "rsi", "type": "rsi", "params": { "period": 14 } }
// ]
// @parameters: [
//   { "id": "session_start", "name": "Session start hour (UTC)", "type": "integer", "default": 0, "min": 0, "max": 23 },
//   { "id": "session_end", "name": "Session end hour (UTC, exclusive)", "type": "integer", "default": 8, "min": 1, "max": 24 },
//   { "id": "rsi_oversold", "name": "RSI Oversold", "type": "number", "default": 30.0 }
// ]

fn on_candle() {
    let h = candle_hour();
    let in_session = h >= param("session_start") && h < param("session_end");
    if !in_session {
        return "hold";
    }

    // candle_time() is Unix seconds — usable for real-elapsed-time gates.
    let t = candle_time();
    if t <= 0 {
        return "hold";
    }

    if crossed_below_value("rsi", "value", param("rsi_oversold")) {
        return #{ signal: "buy", rule_name: "Asian-session RSI fade" };
    }
    "hold"
}
