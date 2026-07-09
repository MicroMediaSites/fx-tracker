//! ABI v5 position state + script-global persistence (AGT-651).
//!
//! Two contracts the rules→rhai converter (`wickd strategy convert`) and the
//! STRATEGY_ABI v5 docs depend on:
//!
//! 1. Top-level `let` globals persist across `on_candle()` calls (the scope
//!    is retained between `call_fn` invocations) — signal-time SL/TP
//!    tracking relies on it.
//! 2. The backtest engine pushes its open-position truth into the script
//!    each candle: `in_position()`, `entry_price()`, `bars_since_entry()`
//!    return real values inside a `BacktestEngine` run and their flat
//!    sentinels (false / 0 / -1) everywhere else.

use rust_decimal_macros::dec;
use wickd_core::backtest::{BacktestConfig, BacktestEngine, ScriptedStrategy, Signal, Strategy};
use wickd_core::models::{Candle, Ohlc};

fn candle(i: i64, close: rust_decimal::Decimal) -> Candle {
    Candle {
        time: chrono::DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc)
            + chrono::Duration::hours(i),
        volume: 100,
        complete: true,
        mid: Ohlc { open: close, high: close + dec!(0.001), low: close - dec!(0.001), close },
    }
}

#[test]
fn globals_persist_across_on_candle_calls() {
    let script = r#"
let counter = 0;
fn on_candle() {
    counter += 1;
    if counter >= 3 { return "buy"; }
    "hold"
}
"#;
    let mut s = ScriptedStrategy::from_script(script, "probe").unwrap();
    let sigs: Vec<Signal> = (0..4).map(|i| s.on_candle(&candle(i, dec!(1.1)))).collect();
    assert_eq!(sigs[3], Signal::Buy, "global counter did not persist: {sigs:?}");
}

#[test]
fn v5_accessors_sit_at_flat_sentinels_without_an_engine() {
    let script = r#"
fn on_candle() {
    if in_position() { return "sell"; }
    if entry_price() != 0.0 { return "sell"; }
    if bars_since_entry() != -1 { return "sell"; }
    "buy"
}
"#;
    let mut s = ScriptedStrategy::from_script(script, "sentinels").unwrap();
    // Driven directly (like the live watcher / `strategy run`): no engine,
    // so every candle must see the flat sentinels.
    for i in 0..3 {
        assert_eq!(
            s.on_candle(&candle(i, dec!(1.1))),
            Signal::Buy,
            "candle {i} saw non-sentinel position state"
        );
    }
}

#[test]
fn engine_feeds_real_position_state_to_scripts() {
    // Buys on the first candle, then closes exactly when the engine reports
    // 3 completed bars since entry.
    let script = r#"
fn on_candle() {
    if in_position() {
        if entry_price() <= 0.0 { return #{ signal: "close", exit_reason: "bad-entry-price" }; }
        if bars_since_entry() >= 3 { return #{ signal: "close", exit_reason: "bars" }; }
        return "hold";
    }
    if bar_count() == 1 { return "buy"; }
    "hold"
}
"#;
    let mut strategy = ScriptedStrategy::from_script(script, "v5").unwrap();
    strategy.set_pip_value_for_instrument("EUR_USD");
    let candles: Vec<Candle> = (0..10).map(|i| candle(i, dec!(1.1))).collect();
    let config = BacktestConfig { initial_balance: dec!(10000), ..Default::default() };
    let result = BacktestEngine::new(config).run(&mut strategy, &candles);

    // Signal on candle 0 (bar_count()==1) → fill at candle 1's open;
    // bars_since_entry advances 0,1,2 then hits 3 on candle 4 → close
    // signal → exit at candle 5's open. Exactly one completed trade, and
    // entry_price() was a real (positive) fill the whole way.
    assert_eq!(result.trades.len(), 1, "trades: {:?}", result.trades);
    let trade = &result.trades[0];
    assert_eq!(trade.exit_reason.as_deref(), Some("bars"), "trade: {trade:?}");
    assert!(trade.entry_time.starts_with("2024-01-01T01"), "expected fill on candle 1, got {}", trade.entry_time);
    let exit_time = trade.exit_time.as_deref().unwrap();
    assert!(exit_time.starts_with("2024-01-01T05"), "expected exit at candle 5 open, got {exit_time}");
}
