// Golden script (STRATEGY_ABI.md "Event calendar" example, ABI v3).
//
// The H-011/H-012 pattern from the edge-hunt registry: RSI-extreme fades
// carry positive expectancy at baseline but NEGATIVE expectancy in the
// 24–72h window after a rate decision or NFP (post-announcement drift), so
// this reversion strategy blacks itself out of post-event windows using
// hours_since_event(). hours_until_event() additionally keeps it flat just
// before a known event. -1 means "no calendar / no event on that side" —
// treated as not-gated.
//
// Honesty caveat (see STRATEGY_ABI.md): backtest spread cost is flat, but
// real spreads blow out around news — event-window results are optimistic.
// @indicators: [
//   { "id": "rsi", "type": "rsi", "params": { "period": 14 } }
// ]
// @parameters: [
//   { "id": "blackout_hours", "name": "Post-event blackout (hours)", "type": "number", "default": 72.0, "min": 0.0, "max": 240.0 },
//   { "id": "pre_event_hours", "name": "Pre-event flat window (hours)", "type": "number", "default": 4.0, "min": 0.0, "max": 48.0 },
//   { "id": "rsi_oversold", "name": "RSI Oversold", "type": "number", "default": 30.0 }
// ]

fn on_candle() {
    let since = hours_since_event();
    if since >= 0.0 && since < param("blackout_hours") {
        return "hold"; // post-event drift window: do not fade extremes
    }
    // NOTE: `until` is a reserved word in Rhai — don't use it as a variable.
    let to_event = hours_until_event();
    if to_event >= 0.0 && to_event < param("pre_event_hours") {
        return "hold"; // event imminent: stay flat
    }

    if crossed_below_value("rsi", "value", param("rsi_oversold")) {
        return #{ signal: "buy", rule_name: "RSI fade (event-gated)" };
    }
    "hold"
}
