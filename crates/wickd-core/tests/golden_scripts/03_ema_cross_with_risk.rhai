// Golden script (STRATEGY_ABI.md example: "Extended signal + risk levels").
//
// Returns the extended-signal object-map form (#{ signal, stop_loss, take_profit,
// rule_name, exit_reason }) instead of a plain string, and implements the
// optional on_position_closed() lifecycle hook.
// @indicators: [
//   { "id": "ema_fast", "type": "ema", "params": { "period": { "$param": "fast_period" } } },
//   { "id": "ema_slow", "type": "ema", "params": { "period": { "$param": "slow_period" } } },
//   { "id": "atr", "type": "atr", "params": { "period": 14 } }
// ]
// @parameters: [
//   { "id": "fast_period", "name": "Fast EMA Period", "type": "integer", "default": 9, "min": 3, "max": 50, "step": 1 },
//   { "id": "slow_period", "name": "Slow EMA Period", "type": "integer", "default": 21, "min": 5, "max": 200, "step": 1 }
// ]

fn on_candle() {
    let close = price("close");
    let atr = indicator("atr", "value");

    if crossed_above("ema_fast", "value", "ema_slow", "value") {
        let sl = close - atr * 1.5;
        let tp = close + atr * 3.0;
        return #{ signal: "buy", stop_loss: sl, take_profit: tp, rule_name: "EMA fast crossed above slow" };
    }

    if crossed_below("ema_fast", "value", "ema_slow", "value") {
        return #{ signal: "close", exit_reason: "EMA fast crossed below slow" };
    }

    #{ signal: "hold" }
}

fn on_position_closed() {
    // Optional lifecycle hook — called when the engine closes a position the
    // script opened (SL/TP fill, forced close, etc.) so any internal state
    // tracking the script keeps can be cleared.
}
