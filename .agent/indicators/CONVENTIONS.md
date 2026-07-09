# Indicators Domain Conventions

## Indicator Struct Pattern

Every indicator follows a consistent structure:

```rust
// Section header comment
// ============================================================================
// Indicator Name
// ============================================================================

pub struct XxxIndicator {
    period: usize,              // Configuration (immutable after construction)
    values: VecDeque<Decimal>,  // Rolling window state
    computed: Option<Decimal>,  // Smoothed/computed state (None = not yet seeded)
}

impl XxxIndicator {
    pub fn new(period: usize) -> Self {
        Self {
            period,
            values: VecDeque::with_capacity(period),
            computed: None,
        }
    }
}

impl Indicator for XxxIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs { ... }
    fn indicator_type(&self) -> &str { "xxx" }
    fn output_names(&self) -> Vec<&str> { vec!["value"] }
    fn reset(&mut self) { ... }
}
```

### Key patterns within `on_candle()`:

1. **Push new data, trim buffer**: Always push first, then trim to capacity.
   ```rust
   self.prices.push_back(candle.mid.close);
   while self.prices.len() > self.period {
       self.prices.pop_front();
   }
   ```

2. **Check warmup before computing**: Return empty HashMap if insufficient data.
   ```rust
   let mut outputs = HashMap::new();
   if self.prices.len() < self.period {
       return outputs;
   }
   ```

3. **Seeding pattern for smoothed indicators**: First value uses simple average, subsequent values use exponential smoothing.
   ```rust
   if self.smoothed.is_none() {
       // Use SMA for initial seed
       let sum: Decimal = self.values.iter().sum();
       self.smoothed = Some(sum / Decimal::from(self.period as u32));
   } else {
       let prev = self.smoothed.unwrap();
       self.smoothed = Some((value - prev) * self.multiplier + prev);
   }
   ```

4. **Composition**: Complex indicators compose simpler ones.
   - `MacdIndicator` owns two `EmaIndicator` instances
   - `ChandelierIndicator` owns an `AtrIndicator`
   - `DssIndicator` performs double EMA smoothing internally

## Naming Conventions

### Indicator Types
- Rust struct: `XxxIndicator` (PascalCase + "Indicator" suffix)
- `IndicatorType` enum variant: `Xxx` (PascalCase, no suffix)
- `indicator_type()` return / `as_str()`: `"xxx"` (snake_case string)
- Serde serialization: `snake_case` (e.g., `ma_histogram`, `ma_bands`)

### Output Names
| Pattern | Output Names | Examples |
|---------|-------------|----------|
| Single value | `"value"` | SMA, EMA, RSI, ATR, ADX, MFI |
| Bands/channels | `"upper"`, `"middle"`, `"lower"` | Bollinger, Donchian, MA Bands |
| Oscillator with signal | `"k"`, `"d"` or `"dss"`, `"signal"` | Stochastic, DSS |
| MACD family | `"macd"`, `"signal"`, `"histogram"` | MACD |
| Ichimoku | `"tenkan"`, `"kijun"`, `"senkou_a"`, `"senkou_b"`, `"cloud_top"`, `"cloud_bottom"`, `"chikou"` | Ichimoku |
| Chandelier | `"exit_long"`, `"exit_short"` | Chandelier Exit |
| ADX directional | `"value"`, `"plus_di"`, `"minus_di"` | ADX |
| Daily stats | `"high"`, `"low"`, `"range"`, `"open"` | Daily |
| ADR | `"value"`, `"ratio"` | ADR |
| Swing | `"recent_high"`, `"recent_high_bars"`, `"recent_low"`, `"recent_low_bars"`, `"prev_high"`, `"prev_high_bars"`, `"prev_low"`, `"prev_low_bars"` | Swing |
| MA Histogram | `"histogram"`, `"fast_ma"`, `"slow_ma"` | MA Histogram |

### Parameter Names
| Param | Usage | Default |
|-------|-------|---------|
| `period` | Lookback window size | Varies (14 for RSI/ATR/ADX/MFI/ADR, 20 for SMA/Bollinger/MA Bands/Donchian, 3 for Stochastic D) |
| `fast_period` / `slow_period` | Dual-MA indicators | MACD: 12/26, MA Histogram: 5/13 |
| `signal_period` | Signal line period | MACD: 9, DSS: 8 |
| `std_dev` | Standard deviation multiplier | Bollinger: 2 |
| `multiplier` | ATR multiplier | Chandelier: 3 |
| `distance` | Pip distance for bands | MA Bands: 20 |
| `tenkan_period` / `kijun_period` / `senkou_b_period` / `displacement` | Ichimoku periods | 9/26/52/26 |
| `k_period` / `d_period` | Stochastic periods | 14/3 |
| `stoch_period` / `ema_period` | DSS periods | 13/8 |
| `strength` | Swing detection bar count | 5 |

## Factory Function Convention (`create_indicator`)

Every `IndicatorType` variant has a corresponding match arm in `create_indicator()`:

```rust
IndicatorType::Xxx => {
    let period = get_param_usize(params, "period", DEFAULT)?;
    Ok(Box::new(XxxIndicator::new(period)))
}
```

- Always use `get_param_usize()` or `get_param_decimal()` with a default value
- The default should match the indicator's conventional standard (e.g., RSI 14, Bollinger 20)
- Parameter names must match what the frontend sends in `IndicatorConfig.params`

## Price Input Convention

- Most indicators use `candle.mid.close` as input
- Band/channel indicators use `candle.mid.high` and `candle.mid.low`
- ATR uses all of high, low, close (plus previous close for True Range)
- MFI uses typical price: `(high + low + close) / 3` and `candle.volume`
- Day-boundary indicators (ADR, Daily) use `candle.time.ordinal()` for day detection

## Testing Conventions

All indicator tests live in a `#[cfg(test)] mod tests` block at the bottom of `indicators.rs` and `indicator_engine.rs`.

### Test helper:
```rust
fn create_test_candle(price: Decimal, time_offset: i64) -> Candle {
    // Creates candle with: open = price - 0.001, high = price + 0.001,
    // low = price - 0.002, close = price, volume = 1000, complete = true
    // Base time: 2024-01-01T00:00:00Z + time_offset hours
}
```

### Required tests for each indicator:
1. **Calculation correctness** -- verify output against known formula
2. **Reset behavior** -- after `reset()`, indicator needs full warmup again
3. **Type and output names** -- `indicator_type()` and `output_names()` return correct values
4. **Bounds checking** -- oscillators stay within 0-100, ATR is positive
5. **Edge cases** -- flat price (zero range), insufficient data returns empty

### Test naming: `test_{indicator}_{what}`
Examples: `test_sma_calculation`, `test_rsi_bounds`, `test_stochastic_flat_range`, `test_bollinger_reset`

## Anti-Patterns

### Do NOT use `f64` for indicator values
All prices, indicator outputs, and intermediate calculations must use `Decimal`. The only place `f64` appears is in `IndicatorConfig.params` (for compatibility with the JSON strategy format) and is immediately converted to `usize` or `Decimal` in the factory function.

### Do NOT panic on missing data
Return an empty `HashMap::new()` when the indicator has insufficient data. Never use `unwrap()` on price data that might not exist -- use `Option` and early returns.

### Do NOT add indicator state that survives `reset()`
After `reset()`, the indicator must behave identically to a freshly constructed instance. Every mutable field must be cleared or restored to its initial value.

### Do NOT add indicator-specific logic to the rules engine
The rules engine should only interact with indicators through `IndicatorEngine::get_output()` and `get_latest()`. If you need new indicator behavior, add it to the indicator itself, not to the consumer.

### Do NOT skip the factory function
Every new indicator MUST have a corresponding `create_indicator()` match arm. The `IndicatorEngine` is the sole creation path for strategy-pipeline indicators. Direct instantiation is reserved for the ai-analysis module's hardcoded context calculations.

### Do NOT add frontend types to indicator Rust files
Indicator calculation logic is pure math. Serialization types (`Serialize`/`Deserialize`) should only appear on output structs meant for API responses (like `PivotLevels`), not on internal calculation state.

### Do NOT change output names for existing indicators
Output names (e.g., `"value"`, `"upper"`, `"macd"`) are part of the public contract. Existing strategies reference these names in their `DataSource::Indicator` configs. Changing an output name would silently break all strategies using that indicator.
