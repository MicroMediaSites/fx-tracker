// Golden script (STRATEGY_ABI.md "SDK function reference" example).
//
// Not a real trading strategy — an ABI smoke test that calls every native SDK
// function documented in STRATEGY_ABI.md at least once, so a function rename or
// removal in scripted_strategy.rs surfaces here as a validate_script failure
// instead of silently going undocumented (or the doc silently going stale).
// @indicators: [
//   { "id": "rsi", "type": "rsi", "params": { "period": 14 } },
//   { "id": "ema_fast", "type": "ema", "params": { "period": 9 } },
//   { "id": "ema_slow", "type": "ema", "params": { "period": 21 } }
// ]
// @parameters: [
//   { "id": "rsi_overbought", "name": "RSI Overbought", "type": "number", "default": 70.0 }
// ]

fn on_candle() {
    let bc = bar_count();
    let vol = volume();
    let pv = pip_value();
    let t = candle_time();
    let h = candle_hour();
    let ev_since = hours_since_event();
    let ev_until = hours_until_event();
    let z = surprise_z();
    let z_med = surprise_z("medium");
    let z_usd = surprise_z_for("USD");
    let z_ago = surprise_hours_ago();
    let z_ago_med = surprise_hours_ago("medium");
    let z_usd_ago = surprise_hours_ago_for("USD");
    let in_pos = in_position();
    let entry = entry_price();
    let bars_held = bars_since_entry();
    let rng = candle_range();
    let body = body_size();
    let c = price("close");
    let c_prev = price_at("close", 1);
    let rsi_now = indicator("rsi", "value");
    let rsi_prev = indicator_at("rsi", "value", 1);
    let overbought = param("rsi_overbought");
    let bullish = is_bullish();
    let bearish = is_bearish();
    let crossed_up = crossed_above("ema_fast", "value", "ema_slow", "value");
    let crossed_down = crossed_below("ema_fast", "value", "ema_slow", "value");
    let broke_overbought = crossed_above_value("rsi", "value", 70.0);
    let broke_oversold = crossed_below_value("rsi", "value", 30.0);

    if crossed_up && rsi_now < overbought {
        return #{ signal: "buy", rule_name: "EMA cross + RSI filter" };
    }
    if crossed_down || broke_overbought {
        return #{ signal: "close", exit_reason: "EMA cross down or RSI overbought" };
    }
    "hold"
}

fn on_position_closed() {
}
