# wickd Indicators Reference

This guide covers all available technical indicators in wickd strategies.

---

## Available Indicator Types (18 total)

| Type | Description | Outputs |
|------|-------------|---------|
| `sma` | Simple Moving Average | `value` |
| `ema` | Exponential Moving Average | `value` |
| `rsi` | Relative Strength Index | `value` |
| `mfi` | Money Flow Index (volume-weighted) | `value` |
| `atr` | Average True Range | `value` |
| `adr` | Average Daily Range | `value`, `ratio` |
| `adx` | Average Directional Index | `value`, `plus_di`, `minus_di` |
| `macd` | Moving Average Convergence Divergence | `macd`, `signal`, `histogram` |
| `bollinger` | Bollinger Bands | `upper`, `middle`, `lower` |
| `stochastic` | Stochastic Oscillator | `k`, `d` |
| `dss` | Double Smoothed Stochastic | `dss`, `signal` |
| `ma_histogram` | Moving Average Histogram | `histogram`, `fast_ma`, `slow_ma` |
| `ma_bands` | Moving Average Bands | `upper`, `middle`, `lower` |
| `ichimoku` | Ichimoku Cloud | `tenkan`, `kijun`, `senkou_a`, `senkou_b`, `chikou`, `cloud_top`, `cloud_bottom` |
| `chandelier` | Chandelier Exit | `exit_long`, `exit_short` |
| `daily` | Current Day's Stats | `high`, `low`, `range`, `open` |
| `swing` | Swing High/Low Detection | `recent_high`, `recent_low`, `prev_high`, `prev_low`, etc. |
| `donchian` | Donchian Channel (N-bar high/low) | `upper`, `middle`, `lower` |

---

## Indicator Configuration

Each indicator is defined in the strategy's `indicators` array:

```json
{
  "indicators": [
    {
      "id": "my_ema",
      "type": "ema",
      "params": { "period": 20 }
    }
  ]
}
```

### Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | Yes | Unique identifier, referenced in triggers |
| `type` | string | Yes | One of the indicator types above |
| `params` | object | Yes | Indicator-specific parameters |
| `symbol` | string | No | Multi-symbol support (not yet implemented) |

---

## Indicator Parameters

### SMA / EMA

```json
{
  "id": "fast_ema",
  "type": "ema",
  "params": { "period": 9 }
}
```

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `period` | number | - | Lookback period |

**Output:** `value`

---

### RSI

```json
{
  "id": "rsi",
  "type": "rsi",
  "params": { "period": 14 }
}
```

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `period` | number | 14 | Lookback period |

**Output:** `value` (0-100 scale)

---

### MFI (Money Flow Index)

```json
{
  "id": "mfi",
  "type": "mfi",
  "params": { "period": 14 }
}
```

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `period` | number | 14 | Lookback period |

**Output:** `value` (0-100 scale, volume-weighted momentum)

**Note:** MFI uses OANDA tick volume data. Similar to RSI but incorporates volume, making it useful for confirming price movements with volume.

---

### ATR

```json
{
  "id": "atr",
  "type": "atr",
  "params": { "period": 14 }
}
```

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `period` | number | 14 | Lookback period |

**Output:** `value` (in price units)

---

### MACD

```json
{
  "id": "macd",
  "type": "macd",
  "params": {
    "fast_period": 12,
    "slow_period": 26,
    "signal_period": 9
  }
}
```

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `fast_period` | number | 12 | Fast EMA period |
| `slow_period` | number | 26 | Slow EMA period |
| `signal_period` | number | 9 | Signal line period |

**Outputs:**
- `macd` - MACD line (fast - slow)
- `signal` - Signal line (EMA of MACD)
- `histogram` - MACD - Signal

---

### Bollinger Bands

```json
{
  "id": "bb",
  "type": "bollinger",
  "params": {
    "period": 20,
    "std_dev": 2.0
  }
}
```

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `period` | number | 20 | SMA period |
| `std_dev` | number | 2.0 | Standard deviation multiplier |

**Outputs:**
- `upper` - Upper band
- `middle` - Middle band (SMA)
- `lower` - Lower band

---

### Stochastic

```json
{
  "id": "stoch",
  "type": "stochastic",
  "params": {
    "k_period": 14,
    "d_period": 3
  }
}
```

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `k_period` | number | 14 | %K lookback period |
| `d_period` | number | 3 | %D smoothing period |

**Outputs:**
- `k` - %K line (0-100)
- `d` - %D line (0-100)

---

### Ichimoku Cloud

```json
{
  "id": "ichimoku",
  "type": "ichimoku",
  "params": {
    "tenkan_period": 9,
    "kijun_period": 26,
    "senkou_b_period": 52,
    "displacement": 26
  }
}
```

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `tenkan_period` | number | 9 | Tenkan-sen period |
| `kijun_period` | number | 26 | Kijun-sen period |
| `senkou_b_period` | number | 52 | Senkou Span B period |
| `displacement` | number | 26 | Cloud displacement |

**Outputs:**
- `tenkan` - Tenkan-sen (Conversion Line)
- `kijun` - Kijun-sen (Base Line)
- `senkou_a` - Senkou Span A (Leading Span A)
- `senkou_b` - Senkou Span B (Leading Span B)
- `chikou` - Chikou Span (Lagging Span)
- `cloud_top` - Top of cloud (max of senkou_a, senkou_b)
- `cloud_bottom` - Bottom of cloud (min of senkou_a, senkou_b)

---

### Chandelier Exit

```json
{
  "id": "chandelier",
  "type": "chandelier",
  "params": {
    "period": 22,
    "multiplier": 3.0
  }
}
```

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `period` | number | 22 | ATR period |
| `multiplier` | number | 3.0 | ATR multiplier |

**Outputs:**
- `exit_long` - Exit level for long positions
- `exit_short` - Exit level for short positions

---

### Donchian Channel

```json
{
  "id": "donchian",
  "type": "donchian",
  "params": {
    "period": 20
  }
}
```

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `period` | number | 20 | Lookback period for high/low |

**Outputs:**
- `upper` - Highest high over N bars
- `middle` - (Upper + Lower) / 2
- `lower` - Lowest low over N bars

**Use Cases:**
- Breakout strategies: "Price crosses above donchian.upper"
- Range boundaries: Mark the high/low of the last N candles
- Channel trading: Buy at lower, sell at upper

---

## Parameterized Indicators

Make indicator parameters optimizable by using parameter references:

```json
{
  "parameters": [
    {
      "id": "ema_period",
      "name": "EMA Period",
      "type": "integer",
      "default": 20,
      "min": 10,
      "max": 50,
      "step": 5
    }
  ],
  "indicators": [
    {
      "id": "my_ema",
      "type": "ema",
      "params": { "period": { "$param": "ema_period" } }
    }
  ]
}
```

This allows the EMA period to be optimized during walk-forward testing.

---

## Referencing Indicators in Triggers

Indicators are referenced by their `id` in trigger data sources:

```json
{
  "type": "compare",
  "left": { "indicator": "my_ema", "output": "value" },
  "operator": ">",
  "right": { "source": "price", "value": "close" }
}
```

### Important Notes

- `indicator` must match an `id` in the `indicators` array
- `output` must be a valid output for that indicator type
- Do NOT use `"type": "indicator"` - indicator sources don't have a type field
- Use `offset` to look back N bars (default: 0 = current candle)

---

## Common Patterns

### EMA Crossover

```json
{
  "indicators": [
    { "id": "fast_ema", "type": "ema", "params": { "period": 9 } },
    { "id": "slow_ema", "type": "ema", "params": { "period": 21 } }
  ],
  "entry_rules": [{
    "conditions": [{
      "primary": {
        "trigger": {
          "type": "cross",
          "left": { "indicator": "fast_ema", "output": "value" },
          "right": { "indicator": "slow_ema", "output": "value" },
          "direction": "above"
        },
        "negated": false
      },
      "chain": []
    }]
  }]
}
```

### RSI Oversold Filter

```json
{
  "indicators": [
    { "id": "rsi", "type": "rsi", "params": { "period": 14 } }
  ],
  "entry_rules": [{
    "conditions": [{
      "primary": {
        "trigger": {
          "type": "compare",
          "left": { "indicator": "rsi", "output": "value" },
          "operator": "<",
          "right": { "fixed": 30 }
        },
        "negated": false
      },
      "chain": []
    }]
  }]
}
```

### Ichimoku TK Cross with Cloud Filter

```json
{
  "indicators": [
    {
      "id": "ichimoku",
      "type": "ichimoku",
      "params": {
        "tenkan_period": 9,
        "kijun_period": 26,
        "senkou_b_period": 52,
        "displacement": 26
      }
    }
  ],
  "entry_rules": [{
    "conditions": [
      {
        "primary": {
          "trigger": {
            "type": "cross",
            "left": { "indicator": "ichimoku", "output": "tenkan" },
            "right": { "indicator": "ichimoku", "output": "kijun" },
            "direction": "above"
          },
          "negated": false
        },
        "chain": []
      },
      {
        "primary": {
          "trigger": {
            "type": "compare",
            "left": { "source": "price", "value": "close" },
            "operator": ">",
            "right": { "indicator": "ichimoku", "output": "cloud_top" }
          },
          "negated": false
        },
        "chain": []
      }
    ]
  }]
}
```

---

## Advanced Features

Beyond indicators, wickd strategies support these powerful features:

### Custom Variables

Create reusable computed values like "TK Gap" (distance between Tenkan/Kijun) that can be referenced in triggers at different offsets.

```json
{
  "variables": [
    {
      "id": "tk_gap",
      "name": "TK Gap",
      "expression": {
        "type": "distance",
        "left": { "indicator": "ichimoku", "output": "tenkan" },
        "right": { "indicator": "ichimoku", "output": "kijun" }
      }
    }
  ]
}
```

**Expression types:**
- `distance` - Difference between two values (signed or absolute)
- `ratio` - Ratio of two values
- `change` - Change over N bars (for velocity/momentum)

Use variables with offset support: `{ "type": "variable", "variable": "tk_gap", "offset": 1 }` compares previous vs current.

See [Strategy Authoring Guide](strategy-authoring.md#variables) for full documentation.

### Multi-Timeframe Analysis

Data sources can specify a different timeframe than the strategy's main timeframe:

```json
{
  "left": {
    "indicator": "ema",
    "output": "value",
    "timeframe": "D"   // Use daily EMA even on M15 chart
  }
}
```

This allows rules like "Daily candle is green" while trading on lower timeframes.

### Swing Detection

The `swing` indicator detects swing highs and lows:

```json
{
  "id": "swings",
  "type": "swing",
  "params": { "strength": 5 }
}
```

**Outputs:** `recent_high`, `recent_low`, `prev_high`, `prev_low` (plus bars-ago counts)

Use for breakout strategies: "Price crosses above recent_high"
