// Golden script (STRATEGY_ABI.md example: "Indicators + parameters").
//
// Declares one indicator whose period is driven by a @parameters entry via the
// {"$param": "..."} reference form, plus two independent numeric parameters used
// directly in on_candle() via param(id).
// @indicators: [{ "id": "rsi", "type": "rsi", "params": { "period": { "$param": "rsi_period" } } }]
// @parameters: [
//   { "id": "rsi_period", "name": "RSI Period", "type": "integer", "default": 14, "min": 5, "max": 50, "step": 1 },
//   { "id": "overbought", "name": "Overbought Threshold", "type": "number", "default": 70.0, "min": 50.0, "max": 95.0 },
//   { "id": "oversold", "name": "Oversold Threshold", "type": "number", "default": 30.0, "min": 5.0, "max": 50.0 }
// ]

fn on_candle() {
    let rsi_val = indicator("rsi", "value");
    let overbought = param("overbought");
    let oversold = param("oversold");

    if rsi_val > overbought {
        "sell"
    } else if rsi_val < oversold {
        "buy"
    } else {
        "hold"
    }
}
