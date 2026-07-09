// Golden script (STRATEGY_ABI.md "Position state" example, ABI v5).
//
// A risk/reward-managed strategy: enter on an RSI dip with an explicit
// stop, then exit when the trade has earned `rr_target` times its risk or
// gone stale — using the engine's ACTUAL position state (in_position(),
// entry_price(), bars_since_entry()) rather than self-tracked
// approximations. The entry emits a `pending_order` map (documented since
// ABI v5, supported by the engine since AGT-607): a buy_limit slightly
// below the signal close, expiring after 3 bars.
//
// On hosts that do not simulate positions (the live watcher, `wickd
// strategy run`) the v5 accessors sit at their flat sentinels:
// in_position() == false, entry_price() == 0, bars_since_entry() == -1.
// @indicators: [
//   { "id": "rsi", "type": "rsi", "params": { "period": 14 } },
//   { "id": "atr", "type": "atr", "params": { "period": 14 } }
// ]
// @parameters: [
//   { "id": "rsi_oversold", "name": "RSI Oversold", "type": "number", "default": 30.0 },
//   { "id": "rr_target", "name": "R:R exit target", "type": "number", "default": 2.0, "min": 0.5, "max": 10.0 },
//   { "id": "max_bars", "name": "Stale exit (bars)", "type": "integer", "default": 20, "min": 1, "max": 500 }
// ]

// Stop distance at signal time — the risk denominator for the RR exit.
let sig_risk = 0.0;

fn on_candle() {
    if in_position() {
        // Exact RR management against the actual fill (ABI v5).
        let profit = price("close") - entry_price();
        if sig_risk > 0.0 && profit >= sig_risk * param("rr_target") {
            return #{ signal: "close", exit_reason: "RR target reached" };
        }
        if bars_since_entry() >= param("max_bars") {
            return #{ signal: "close", exit_reason: "stale" };
        }
        return "hold";
    }

    if crossed_below_value("rsi", "value", param("rsi_oversold")) {
        let limit = price("close") - indicator("atr", "value") * 0.25;
        let sl = price("close") - indicator("atr", "value") * 1.5;
        sig_risk = limit - sl;
        return #{
            signal: "buy",
            rule_name: "RSI dip (limit entry)",
            stop_loss: sl,
            pending_order: #{ order_type: "buy_limit", price: limit, expiry_bars: 3 }
        };
    }
    "hold"
}

fn on_position_closed() {
    sig_risk = 0.0;
}
