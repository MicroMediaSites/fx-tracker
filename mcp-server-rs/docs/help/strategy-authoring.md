# wickd Strategy Authoring Guide (V2)

A comprehensive guide for creating trading strategies in wickd. This document explains every field, type, and pattern available in the V2 strategy schema.

## Quick Capabilities Summary

**What wickd strategies support:**

| Feature | Supported | Notes |
|---------|-----------|-------|
| **17 Technical Indicators** | ✅ | SMA, EMA, RSI, MFI, ATR, ADR, ADX, MACD, Bollinger, Stochastic, DSS, MA Histogram, MA Bands, Ichimoku, Chandelier, Daily, Swing |
| **Price Comparisons** | ✅ | Open, High, Low, Close with offset (bars back) |
| **Threshold Triggers** | ✅ | RSI > 50, MFI > 50, etc. |
| **Cross Triggers** | ✅ | EMA crosses above/below, price crosses level |
| **Compare Triggers** | ✅ | Any indicator vs indicator, price vs indicator, fixed values |
| **Custom Variables** | ✅ | Create reusable computed values (distance, ratio, change) |
| **Multi-Timeframe** | ✅ | Set `timeframe` on indicator configs or data sources to use different timeframes (e.g., Daily EMA + H1 RSI) |
| **Swing Detection** | ✅ | Recent/previous swing highs and lows |
| **Fixed SL/TP** | ✅ | Pip-based or ATR-based stop loss and take profit |
| **Parameterization** | ✅ | Make any value optimizable for walk-forward testing |
| **Trendlines** | ❌ | No automatic trendline drawing/detection |
| **Chart Patterns** | ❌ | No head & shoulders, triangles, etc. |
| **Fundamental Data** | ❌ | No economic calendar integration in strategies |

**Not sure if something is supported?** Check the [Indicators Reference](indicators.md) and sections below.

---

## ⚠️ CRITICAL: Indicators Array Required

**Every strategy that uses indicators MUST define them in the `indicators` array.**

Triggers reference indicators by ID only - they CANNOT define indicators inline. Any `{ "indicator": "some_id", ... }` reference in a trigger MUST have a matching entry in the `indicators` array.

**Without the indicators array, indicator lookups return null and the strategy will produce ZERO trades.**

```json
{
  "indicators": [
    { "id": "my_ema", "type": "ema", "params": { "period": 20 } }
  ],
  "entry_rules": [{
    "conditions": [{
      "primary": {
        "trigger": {
          "type": "compare",
          "left": { "indicator": "my_ema", "output": "value" },
          "operator": ">",
          "right": { "source": "price", "value": "close" }
        },
        "negated": false
      },
      "chain": []
    }]
  }]
}
```

### Validation Checklist
- [ ] Every indicator ID referenced in triggers exists in the `indicators` array
- [ ] The `indicators` array is passed to `create_strategy` tool (not just in entry_rules)
- [ ] Each indicator has: `id`, `type`, and `params`

### Required for Walk-Forward Testing (WFT)

To run walk-forward optimization, your strategy MUST have:

1. **`indicators` array** - for indicator lookups to work
2. **`parameters` array** - with at least one parameter defined
3. **`{ "$param": "param_id" }` references** - in indicators or triggers to connect params

**Note:** Parameter ranges (min/max/step) are configured in the backtest panel when running Walk-Forward, not in the strategy definition. The strategy just defines what parameters exist with their default values.

Example with parameterized Ichimoku periods:
```json
{
  "parameters": [
    { "id": "tenkan", "name": "Tenkan Period", "type": "integer", "default": 9 }
  ],
  "indicators": [
    { "id": "ichi", "type": "ichimoku", "params": { "tenkan_period": { "$param": "tenkan" }, "kijun_period": 26, ... } }
  ]
}
```

Without parameters, the strategy can only run single backtests, not optimization.

---

## ⚠️ CRITICAL: V2 Format Required (NOT V1)

Entry rules MUST use `conditions`, NOT `trigger` or `trigger_chain`. The presence of `weight` or `required` fields indicates V1 format which will NOT work.

**❌ WRONG (V1 format - will fail):**
```json
{
  "entry_rules": [{
    "id": "rule_1",
    "trigger": { "type": "cross", ... },
    "weight": 10,
    "required": true
  }]
}
```

**✅ CORRECT (V2 format):**
```json
{
  "entry_rules": [{
    "id": "rule_1",
    "direction": "long",
    "conditions": [{
      "primary": {
        "trigger": { "type": "cross", ... },
        "negated": false
      },
      "chain": []
    }]
  }]
}
```

Key V2 requirements:
- Use `conditions` array with `primary` (TriggerWithNot) and `chain` array
- Each trigger is wrapped in `TriggerWithNot` with `negated` boolean
- Use `direction` field ("long", "short", or "both")
- Do NOT use `weight`, `required`, or `entry_logic`

---

## ⚠️ CRITICAL: Data Source Format Differences

Different data sources use different field names. This is a common source of errors:

| Data Source | Key Fields | Example |
|-------------|-----------|---------|
| **Price** | `source: "price"` (NOT type!) | `{"source": "price", "value": "close"}` |
| **Indicator** | `indicator`, `output` (NO type field!) | `{"indicator": "ema_9", "output": "value"}` |
| **Fixed** | `fixed` (number only!) | `{"fixed": 50}` |
| **Parameter** | `$param` (use directly, NOT in fixed!) | `{"$param": "rsi_threshold"}` |
| **Variable** | `type: "variable"` | `{"type": "variable", "variable": "tk_gap"}` |
| **S/R Zone** | `type: "sr_zone"` | `{"type": "sr_zone", "target": "upper"}` |
| **Pivot** | `type: "pivot"` | `{"type": "pivot", "level": "r1"}` |

**❌ COMMON MISTAKES** - These will cause "data did not match any variant of untagged enum DataSource" error:

```json
// WRONG - PriceSource does NOT use "type"
{"type": "price", "value": "close"}
// CORRECT
{"source": "price", "value": "close"}

// WRONG - IndicatorSource does NOT use "type"
{"type": "indicator", "indicator": "ichimoku", "output": "kijun"}
// CORRECT
{"indicator": "ichimoku", "output": "kijun"}

// WRONG - $param CANNOT be wrapped in fixed
{"fixed": {"$param": "threshold"}}
// CORRECT - use $param directly as a data source
{"$param": "threshold"}
```

**Note:** Only Variable, S/R Zone, and Pivot sources use the `"type"` field. Price uses `"source"`, Indicator uses neither, and `$param` is used directly.

---

## Table of Contents

1. [Strategy Structure Overview](#strategy-structure-overview)
2. [Complete Example Strategy](#complete-example-strategy)
3. [Entry Rules](#entry-rules)
4. [Exit Rules](#exit-rules)
5. [Conditions (AND Logic Between Conditions)](#conditions-and-logic-between-conditions)
6. [Trigger Types](#trigger-types)
7. [Data Sources](#data-sources)
8. [Capturing Values at Entry](#capturing-values-at-entry-exit-rules-only)
9. [Indicators](#indicators)
10. [Variables](#variables)
11. [Market Conditions (Givens)](#market-conditions-givens)
12. [Risk Settings](#risk-settings)
13. [Parameters (for Optimization)](#parameters-for-optimization)
14. [Common Patterns](#common-patterns)

---

## Strategy Structure Overview

A V2 strategy is a JSON object with the following top-level fields:

```typescript
interface StrategyV2 {
  schema_version: 2;                  // Always 2 for V2 strategies
  id: string;                         // Auto-generated UUID
  user_id: string;                    // Owner's user ID
  name: string;                       // Display name
  description: string;                // What the strategy does

  indicators: IndicatorDefinition[];  // Indicator definitions (referenced by ID in rules)
  variables?: StrategyVariable[];     // Named computed values (e.g., TK Gap)
  parameters: ParameterDefinition[];  // Optimizable parameters
  entry_rules: EntryRuleV2[];         // Conditions to enter trades
  exit_rules: ExitRuleV2[];           // Conditions to exit trades
  risk_settings: RiskSettings;        // Position sizing and SL/TP

  pivot_config?: PivotConfig;         // Optional pivot point settings
  version: number;                    // Increments on edit
  is_active: boolean;                 // false = deleted
  is_promoted: boolean;               // true = live trading enabled
  is_locked: boolean;                 // true = cannot edit (was promoted)
  is_archived: boolean;               // true = hidden from list
}
```

### Key Concepts

| Concept | Description |
|---------|-------------|
| **Rules** | Entry/Exit rules. Multiple rules are OR'd (any matching rule triggers) |
| **Conditions** | Within a rule. Multiple conditions are AND'd (all must be true) |
| **Triggers** | Within a condition. Combined with AND/OR operators, each has NOT option |

---

## Complete Example Strategy

Here's a fully working V2 strategy using EMA crossover with RSI filter:

```json
{
  "schema_version": 2,
  "id": "strategy_001",
  "user_id": "user_123",
  "name": "EMA Crossover + RSI Filter",
  "description": "Enter long when fast EMA crosses above slow EMA and RSI is not overbought",

  "indicators": [
    {
      "id": "fast_ema",
      "type": "ema",
      "params": { "period": { "$param": "ema_fast" } }
    },
    {
      "id": "slow_ema",
      "type": "ema",
      "params": { "period": { "$param": "ema_slow" } }
    },
    {
      "id": "rsi",
      "type": "rsi",
      "params": { "period": 14 }
    }
  ],

  "parameters": [
    {
      "id": "ema_fast",
      "name": "Fast EMA Period",
      "type": "integer",
      "default": 9,
      "group": "indicator"
    },
    {
      "id": "ema_slow",
      "name": "Slow EMA Period",
      "type": "integer",
      "default": 21,
      "group": "indicator"
    },
    {
      "id": "rsi_threshold",
      "name": "RSI Overbought",
      "type": "integer",
      "default": 70,
      "group": "entry"
    }
  ],

  "entry_rules": [
    {
      "id": "rule_1",
      "name": "EMA Cross + RSI Filter",
      "direction": "long",
      "conditions": [
        {
          "primary": {
            "trigger": {
              "type": "cross",
              "left": { "indicator": "fast_ema", "output": "value" },
              "right": { "indicator": "slow_ema", "output": "value" },
              "direction": "above"
            },
            "negated": false
          },
          "chain": [
            {
              "operator": "and",
              "trigger": {
                "trigger": {
                  "type": "compare",
                  "left": { "indicator": "rsi", "output": "value" },
                  "operator": "<",
                  "right": { "$param": "rsi_threshold" }
                },
                "negated": false
              }
            }
          ]
        }
      ]
    }
  ],

  "exit_rules": [
    {
      "id": "exit_1",
      "name": "Take Profit at R2",
      "direction": "both",
      "close_percent": 100,
      "priority": 1,
      "conditions": [
        {
          "primary": {
            "trigger": {
              "type": "compare",
              "left": { "source": "price", "value": "close" },
              "operator": "is_within",
              "right": { "type": "pivot", "level": "r2" },
              "distance": { "value": 10, "unit": "pips" }
            },
            "negated": false
          },
          "chain": []
        }
      ]
    }
  ],

  "risk_settings": {
    "risk_method": "percent",
    "risk_value": 1,
    "rr_ratio": 2.0,
    "spread_buffer_pips": 1
  },

  "version": 1,
  "is_active": true,
  "is_promoted": false,
  "is_locked": false,
  "is_archived": false
}
```

---

## Entry Rules

Entry rules define when to open a position. **Multiple entry rules are OR'd** - any matching rule can trigger an entry.

```typescript
interface EntryRuleV2 {
  id: string;                              // Unique ID
  name?: string;                           // Display name
  direction: 'long' | 'short' | 'both';    // Which direction
  conditions: Condition[];                 // All conditions must be true (AND logic)
}
```

### Direction Explained

- `"long"`: This rule only applies to long entries
- `"short"`: This rule only applies to short entries
- `"both"`: This rule applies to BOTH directions (e.g., volatility filters)

---

## Exit Rules

Exit rules define **custom** conditions to close a position.

### Automatic SL/TP (No Exit Rules Needed)

**IMPORTANT:** Basic stop loss and take profit are handled automatically by `risk_settings`:
- Stop loss is calculated using ATR
- Take profit is derived from `rr_ratio` (e.g., `rr_ratio: 2` = 2:1 reward)

**You do NOT need exit rules for basic SL/TP.** Just set `risk_settings.rr_ratio` and the backend handles it.

### When to Use Exit Rules

Use exit rules only for **custom exits** like:
- Indicator-based exits (e.g., exit when Tenkan crosses below Kijun)
- Partial profit taking at specific levels
- Time-based exits
- Trailing stops based on indicator values

```typescript
interface ExitRuleV2 {
  id: string;
  name?: string;
  direction: 'long' | 'short' | 'both';
  conditions: Condition[];         // All conditions must be true (AND logic)
  close_percent: number;           // 1-100, how much to close
  priority?: number;               // Higher = evaluated first
}
```

### Partial Exits

Use `close_percent` less than 100 for partial exits:

```json
{
  "id": "partial_tp",
  "name": "Take 50% at 1R",
  "direction": "both",
  "close_percent": 50,
  "priority": 1,
  "conditions": [{ ... }]
}
```

---

## Conditions (AND Logic Between Conditions)

Each rule contains an array of conditions. **All conditions must be true** for the rule to fire (AND logic between conditions).

Within each condition, you can chain multiple triggers with AND/OR logic. Each trigger also has a `negated` flag for NOT logic.

```typescript
interface Condition {
  primary: TriggerWithNot;           // The main trigger (with NOT option)
  chain: ChainedTriggerWithNot[];    // Additional AND/OR triggers
}

interface TriggerWithNot {
  trigger: TriggerV2;                // The trigger itself
  negated: boolean;                  // If true, trigger must be FALSE
}

interface ChainedTriggerWithNot {
  operator: 'and' | 'or';            // How to combine with previous
  trigger: TriggerWithNot;           // The trigger with NOT option
}
```

### Example: RSI < 70 AND Price > EMA

First define indicators:
```json
{
  "indicators": [
    { "id": "rsi", "type": "rsi", "params": { "period": 14 } },
    { "id": "ema200", "type": "ema", "params": { "period": 200 } }
  ]
}
```

Then use a condition with chained triggers:
```json
{
  "conditions": [{
    "primary": {
      "trigger": {
        "type": "compare",
        "left": { "indicator": "rsi", "output": "value" },
        "operator": "<",
        "right": { "fixed": 70 }
      },
      "negated": false
    },
    "chain": [
      {
        "operator": "and",
        "trigger": {
          "trigger": {
            "type": "compare",
            "left": { "source": "price", "value": "close" },
            "operator": ">",
            "right": { "indicator": "ema200", "output": "value" }
          },
          "negated": false
        }
      }
    ]
  }]
}
```

### Example: RSI NOT overbought (using NOT)

```json
{
  "conditions": [{
    "primary": {
      "trigger": {
        "type": "compare",
        "left": { "indicator": "rsi", "output": "value" },
        "operator": ">",
        "right": { "fixed": 70 }
      },
      "negated": true
    },
    "chain": []
  }]
}
```

When `negated: true`, the trigger result is inverted - the condition passes when RSI is NOT > 70.

### Example: Price near S1 OR Price near S2

```json
{
  "conditions": [{
    "primary": {
      "trigger": {
        "type": "compare",
        "left": { "source": "price", "value": "close" },
        "operator": "is_within",
        "right": { "type": "pivot", "level": "s1" },
        "distance": { "value": 15, "unit": "pips" }
      },
      "negated": false
    },
    "chain": [
      {
        "operator": "or",
        "trigger": {
          "trigger": {
            "type": "compare",
            "left": { "source": "price", "value": "close" },
            "operator": "is_within",
            "right": { "type": "pivot", "level": "s2" },
            "distance": { "value": 15, "unit": "pips" }
          },
          "negated": false
        }
      }
    ]
  }]
}
```

### Example: Multiple Conditions (all must be true)

```json
{
  "conditions": [
    {
      "primary": {
        "trigger": { "type": "givens", "regime": "trending_up" },
        "negated": false
      },
      "chain": []
    },
    {
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
    }
  ]
}
```

Both conditions must be true: trending up AND EMA cross.

---

## Trigger Types

**Valid trigger types:**

| Type | Purpose | Use In |
|------|---------|--------|
| `compare` | Compare two values (>, <, >=, <=, ==, !=, is_within) | Entry & Exit |
| `cross` | Detect when one value crosses above/below another | Entry & Exit |
| `givens` | Check market conditions (trending, ranging, volatility) | Entry & Exit |
| `threshold` | Check if value is above/below a threshold | Entry & Exit |
| `risk_reward_reached` | Exit when R:R ratio reached (e.g., 2 = 2R profit) | Exit only |
| `percent_of_tp_reached` | Exit at percentage of TP target | Exit only |
| `time` | Time-based exit (bars since entry) | Exit only |

### Exit-Only Triggers

These require position context and only work in exit rules:

```json
{
  "type": "risk_reward_reached",
  "ratio": 2
}
```

```json
{
  "type": "percent_of_tp_reached",
  "percent": 50
}
```

```json
{
  "type": "time",
  "condition": "bar_count",
  "value": 10
}
```

**Valid time conditions:**
- `bar_count` - Number of bars since entry
- `minutes` - Minutes since entry
- `hours` - Hours since entry

### Givens (Market Conditions)

Check for predefined market regimes:

```json
{
  "type": "givens",
  "regime": "trending_up"
}
```

**Available regimes:**

| Regime | Detection Logic |
|--------|-----------------|
| `trending_up` | ADX > 25, price > SMA20 > SMA50 |
| `trending_down` | ADX > 25, price < SMA20 < SMA50 |
| `ranging` | ADX < 20, Bollinger Band width contracted |
| `high_volatility` | ATR > 1.5x rolling average ATR |
| `low_volatility` | ATR < 0.5x rolling average ATR |
| `sr_tested` | Price within 20 pips of user's S/R zone |

### Cross Trigger

Detects when one value crosses another. First define indicators:

```json
{
  "indicators": [
    { "id": "fast_ema", "type": "ema", "params": { "period": 9 } },
    { "id": "slow_ema", "type": "ema", "params": { "period": 21 } }
  ]
}
```

Then use the cross trigger:

```json
{
  "type": "cross",
  "left": { "indicator": "fast_ema", "output": "value" },
  "right": { "indicator": "slow_ema", "output": "value" },
  "direction": "above"
}
```

- `direction`: `"above"` (left crosses above right) or `"below"`

### Compare Trigger

Compares two values. First define the indicator:

```json
{
  "indicators": [
    { "id": "rsi", "type": "rsi", "params": { "period": 14 } }
  ]
}
```

Then use the compare trigger:

```json
{
  "type": "compare",
  "left": { "indicator": "rsi", "output": "value" },
  "operator": "<",
  "right": { "fixed": 30 }
}
```

**Operators:**
- `>` - greater than
- `<` - less than
- `>=` - greater or equal
- `<=` - less or equal
- `==` - equals
- `!=` - not equals
- `is_within` - within a distance (requires `distance` field)

### "is_within" Operator

For distance-based comparisons (near S/R zones, pivots, etc.):

```json
{
  "type": "compare",
  "left": { "source": "price", "value": "close" },
  "operator": "is_within",
  "right": { "type": "sr_zone", "target": "upper" },
  "distance": {
    "value": 10,
    "unit": "pips"
  }
}
```

**Distance units:**
- `pips` - fixed pip distance
- `atr` - multiple of ATR (e.g., 0.5 = half ATR)
- `percent` - percentage of price

---

## Data Sources

Data sources define WHERE a value comes from.

### Indicator Reference

⚠️ **CRITICAL**: IndicatorSource does NOT use a `"type"` field. This is a common mistake!

Reference an indicator defined in the strategy's `indicators` array:

```json
{
  "indicator": "my_rsi",
  "output": "value",
  "offset": 0
}
```

**❌ WRONG** (will fail parsing):
```json
{
  "type": "indicator",
  "indicator": "my_rsi",
  "output": "value"
}
```

**✅ CORRECT**:
```json
{
  "indicator": "my_rsi",
  "output": "value"
}
```

Fields:
- `indicator` - ID of an indicator in the `indicators` array (required)
- `output` - Which output to use, e.g., `"value"`, `"upper"`, `"tenkan"` (required)
- `offset` - Bars back, 0 = current candle (optional, default 0)

**This applies to DataSourceV2 in TRIGGERS only**, including:
- Trigger `left`/`right` fields in cross/compare triggers

⚠️ **EXCEPTION**: `stop_loss_source` in risk_settings uses a DIFFERENT schema (`StopLossSource`) that DOES require `"type": "indicator"`:
```json
// CORRECT for stop_loss_source (StopLossSource - tagged enum)
"stop_loss_source": { "type": "indicator", "indicator": "ichimoku", "output": "kijun" }

// CORRECT for triggers (DataSourceV2 - untagged enum)
"left": { "indicator": "ichimoku", "output": "kijun" }
```

### Price Source

⚠️ **CRITICAL**: PriceSource uses `"source": "price"`, NOT `"type": "price"`. This is a common mistake!

```json
{
  "source": "price",
  "value": "close"
}
```

**❌ WRONG** (will fail parsing):
```json
{
  "type": "price",
  "value": "close"
}
```

**✅ CORRECT**:
```json
{
  "source": "price",
  "value": "close"
}
```

Values: `"open"`, `"high"`, `"low"`, `"close"`

Optional fields:
- `offset` - Bars back (default: 0)
- `capture` - When to capture: `"each_candle"` (default) or `"at_entry"` (exit rules only)

### Fixed Value

```json
{
  "fixed": 50
}
```

### Parameter Reference

Use directly (NOT wrapped in fixed):
```json
{
  "$param": "rsi_threshold"
}
```

⚠️ **WRONG**: `{"fixed": {"$param": "id"}}` - This will fail! The `fixed` field only accepts numbers.

### S/R Zone Source

```json
{
  "type": "sr_zone",
  "target": "upper"
}
```

Targets: `"upper"`, `"lower"`, `"midpoint"`

Uses the nearest active S/R zone for the instrument.

### Pivot Source

```json
{
  "type": "pivot",
  "level": "r1"
}
```

Levels: `"pp"`, `"r1"`, `"r2"`, `"r3"`, `"s1"`, `"s2"`, `"s3"`

### Variable Source

Reference a named variable defined in the strategy's `variables` array. Variables let you create reusable computed values (like "TK Gap") that can be referenced in conditions at different offsets.

**Structure:**
```json
{
  "type": "variable",
  "variable": "tk_gap",
  "offset": 1
}
```

**Fields:**
- `type` (required): Must be `"variable"`
- `variable` (required): ID of a variable defined in the `variables` array
- `offset` (optional): Bars back to evaluate (default: 0 = current candle)

Variables are defined in the strategy's `variables` array. See [Variables](#variables) for full documentation.

---

## Capturing Values at Entry (Exit Rules Only)

By default, indicator and price values are evaluated fresh on each candle. For **exit rules**, you can **capture the value at trade entry** and use that fixed reference throughout the trade.

This is useful for strategies like:
- Setting stop loss at Kijun line level **when trade opens** (not dynamic)
- Using the ATR value at entry for position sizing
- Referencing the entry candle's high/low for trailing

### Capture Mode

Add `capture` to any indicator or price data source in an **exit rule**:

```json
{
  "indicator": "ichimoku",
  "output": "kijun",
  "capture": "at_entry"
}
```

**Capture modes:**
- `"each_candle"` - (default) Evaluate fresh on each candle
- `"at_entry"` - Capture value when trade opens, use as fixed reference

### Trailing

When using `capture: "at_entry"`, you can optionally enable trailing:

```json
{
  "indicator": "ichimoku",
  "output": "kijun",
  "capture": "at_entry",
  "trail": {
    "enabled": true,
    "percent": 5
  }
}
```

**Trail behavior:**
- For **long positions**: The captured value trails UP (follows indicator if it moves higher)
- For **short positions**: The captured value trails DOWN (follows indicator if it moves lower)
- `percent` (optional): Limits trailing to X% of the initial captured value

### Example: Stop Loss at Entry Kijun

This exit rule closes the position when price falls below the Kijun level that was captured when the trade opened:

```json
{
  "indicators": [
    {
      "id": "ichimoku",
      "type": "ichimoku",
      "params": { "tenkan_period": 9, "kijun_period": 26, "senkou_b_period": 52, "displacement": 26 }
    }
  ],
  "exit_rules": [
    {
      "id": "sl_kijun",
      "name": "Stop at Entry Kijun",
      "direction": "long",
      "close_percent": 100,
      "priority": 1,
      "conditions": [{
        "primary": {
          "trigger": {
            "type": "compare",
            "left": { "source": "price", "value": "close" },
            "operator": "<",
            "right": {
              "indicator": "ichimoku",
              "output": "kijun",
              "capture": "at_entry"
            }
          },
          "negated": false
        },
        "chain": []
      }]
    }
  ]
}
```

In this example, when a long trade opens, the Kijun value is captured. The exit rule then compares price against this **fixed captured value** (not the dynamic current Kijun).

---

## Indicators

Indicators are defined in the strategy's `indicators` array and referenced by ID in triggers.

### Indicator Definition

```typescript
interface IndicatorDefinition {
  id: string;                    // Unique ID, referenced in triggers
  type: IndicatorType;           // e.g., "ema", "rsi", "ichimoku"
  params: Record<string, ParameterizedValue>;  // Indicator parameters
  timeframe?: string;            // Optional: "D", "H4", "W", etc. Defaults to strategy's primary timeframe
}
```

### Example

```json
{
  "indicators": [
    {
      "id": "fast_ema",
      "type": "ema",
      "params": { "period": 9 }
    },
    {
      "id": "slow_ema",
      "type": "ema",
      "params": { "period": { "$param": "slow_period" } }
    },
    {
      "id": "cloud",
      "type": "ichimoku",
      "params": {
        "tenkan_period": 9,
        "kijun_period": 26,
        "senkou_b_period": 52,
        "displacement": 26
      }
    }
  ]
}
```

### Multi-Timeframe Example

Use the `timeframe` field to run indicators on different timeframes. For example, a strategy on H1 that also uses a Daily EMA for trend filtering:

```json
{
  "indicators": [
    {
      "id": "fast_ema",
      "type": "ema",
      "params": { "period": 9 }
    },
    {
      "id": "daily_ema",
      "type": "ema",
      "params": { "period": 50 },
      "timeframe": "D"
    },
    {
      "id": "h4_rsi",
      "type": "rsi",
      "params": { "period": 14 },
      "timeframe": "H4"
    }
  ]
}
```

In this example, `fast_ema` runs on the strategy's primary timeframe (e.g., H1), while `daily_ema` runs on the Daily timeframe and `h4_rsi` runs on H4. All are referenced by ID in triggers the same way — the engine handles timeframe alignment automatically.

### Available Indicator Types

| Type | Parameters | Outputs |
|------|------------|---------|
| `sma` | `period` | `value` |
| `ema` | `period` | `value` |
| `rsi` | `period` | `value` |
| `atr` | `period` | `value` |
| `macd` | `fast_period`, `slow_period`, `signal_period` | `macd`, `signal`, `histogram` |
| `bollinger` | `period`, `std_dev` | `upper`, `middle`, `lower` |
| `stochastic` | `k_period`, `d_period` | `k`, `d` |
| `ichimoku` | `tenkan_period`, `kijun_period`, `senkou_b_period`, `displacement` | `tenkan`, `kijun`, `senkou_a`, `senkou_b`, `chikou`, `cloud_top`, `cloud_bottom` |
| `chandelier` | `period`, `multiplier` | `exit_long`, `exit_short` |

### Parameterized Indicator

Make indicator params optimizable by referencing parameters:

```json
{
  "id": "my_ema",
  "type": "ema",
  "params": {
    "period": { "$param": "ema_period" }
  }
}
```

Then reference in triggers:
```json
{
  "indicator": "my_ema",
  "output": "value"
}
```

---

## Variables

Variables let you define named computed values that can be referenced in entry/exit conditions. They're useful for:
- **Naming complex calculations**: Create "TK Gap" instead of inline distance expressions
- **Reusing computations**: Reference the same variable at different offsets
- **Detecting convergence/divergence**: Compare a value at different time points

### Variable Definition

```typescript
interface StrategyVariable {
  id: string;           // Unique ID, referenced in triggers
  name: string;         // Display name
  description?: string; // Optional description
  expression: VariableExpression;
}
```

### Expression Types

Variables support three expression types:

#### Distance Expression

Computes the difference between two data sources:

```json
{
  "id": "tk_gap",
  "name": "TK Gap",
  "description": "Distance between Tenkan and Kijun",
  "expression": {
    "type": "distance",
    "left": { "indicator": "ichimoku", "output": "tenkan" },
    "right": { "indicator": "ichimoku", "output": "kijun" },
    "absolute": false
  }
}
```

Fields:
- `left`: First data source
- `right`: Second data source
- `absolute`: If `true`, returns `|left - right|`. Default: `false` (signed difference)

#### Ratio Expression

Computes the ratio of two values:

```json
{
  "id": "rsi_ratio",
  "name": "RSI to Threshold Ratio",
  "expression": {
    "type": "ratio",
    "numerator": { "indicator": "rsi", "output": "value" },
    "denominator": { "fixed": 50 }
  }
}
```

Fields:
- `numerator`: Value to divide
- `denominator`: Value to divide by

#### Change Expression

Computes the change in a value over N bars:

```json
{
  "id": "tk_velocity",
  "name": "TK Gap Velocity",
  "description": "How fast TK gap is changing",
  "expression": {
    "type": "change",
    "source": { "type": "variable", "variable": "tk_gap" },
    "bars": 3
  }
}
```

Fields:
- `source`: The value to measure change for
- `bars`: How many bars back to compare (calculates `value[N] - value[0]`)

**Note:** Positive change = value was higher in the past (declining). Negative change = value was lower in the past (rising).

### Referencing Variables

Use variables as data sources in triggers with offset support:

```json
{
  "type": "compare",
  "left": { "type": "variable", "variable": "tk_gap", "offset": 1 },
  "operator": ">",
  "right": { "type": "variable", "variable": "tk_gap", "offset": 0 }
}
```

This compares `tk_gap[1]` (previous candle) to `tk_gap[0]` (current candle). If previous > current, the lines are converging.

### Complete Example: Filter Weak Tenkan/Kijun Crosses

Avoid trades where Tenkan and Kijun were running parallel before crossing:

```json
{
  "indicators": [
    {
      "id": "ichimoku",
      "type": "ichimoku",
      "params": { "tenkan_period": 9, "kijun_period": 26, "senkou_b_period": 52, "displacement": 26 }
    }
  ],
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
  ],
  "entry_rules": [{
    "id": "strong_tk_cross",
    "name": "TK Cross with Convergence Filter",
    "direction": "long",
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
            "left": { "type": "variable", "variable": "tk_gap", "offset": 1 },
            "operator": ">",
            "right": { "type": "variable", "variable": "tk_gap", "offset": 0 }
          },
          "negated": false
        },
        "chain": []
      }
    ]
  }]
}
```

This strategy:
1. Requires Tenkan crosses above Kijun
2. Requires the previous TK gap to be greater than the current gap (convergence before cross)

If the lines were parallel (same gap), the second condition fails and the entry is filtered out.

---

## Market Conditions (Givens)

Givens are predefined market conditions detected by the backend. Use them as entry filters.

### Example: Only trade when trending

First define the indicator:
```json
{
  "indicators": [
    { "id": "fast_ema", "type": "ema", "params": { "period": 9 } }
  ]
}
```

Then use in conditions:
```json
{
  "conditions": [
    {
      "primary": {
        "trigger": { "type": "givens", "regime": "trending_up" },
        "negated": false
      },
      "chain": []
    },
    {
      "primary": {
        "trigger": {
          "type": "cross",
          "left": { "indicator": "fast_ema", "output": "value" },
          "right": { "source": "price", "value": "close" },
          "direction": "above"
        },
        "negated": false
      },
      "chain": []
    }
  ]
}
```

### Available Market Conditions

**Trend & Volatility:**
- `trending_up` - Strong uptrend (ADX > 25, aligned SMAs)
- `trending_down` - Strong downtrend
- `ranging` - Consolidating market (ADX < 20)
- `high_volatility` - ATR > 1.5x average
- `low_volatility` - ATR < 0.5x average

**Support & Resistance:**
- `sr_tested` - Price near user's S/R zone

---

## Risk Settings

```typescript
interface RiskSettings {
  risk_method: 'percent' | 'fixed';
  risk_value: ParameterizedValue;
  rr_ratio: ParameterizedValue;
  spread_buffer_pips: ParameterizedValue;
  stop_loss_source?: StopLossSource;  // NEW: Custom stop loss source
}

type StopLossSource =
  | { type: 'auto' }  // Default: Chandelier → ATR → 2% fallback
  | { type: 'indicator'; indicator: string; output: string; capture?: CaptureMode }
  | { type: 'atr_multiplier'; multiplier: ParameterizedValue }
  | { type: 'fixed_pips'; pips: ParameterizedValue };
```

### Example (Basic)

```json
{
  "risk_method": "percent",
  "risk_value": 1,
  "rr_ratio": 2.0,
  "spread_buffer_pips": 1
}
```

### Stop Loss Source

The `stop_loss_source` field determines how the **conceptual stop level** is calculated. This affects:
- **Position sizing**: Risk per trade calculation
- **R:R triggers**: The `risk_reward_reached` trigger uses this stop for R calculation
- **Take profit**: Derived from R:R ratio × stop distance

By default, the system uses `"auto"` which tries Chandelier exit first, then ATR-based stop, then 2% fallback.

#### Example: Use Kijun as Stop Loss Source

If you want `risk_reward_reached` to use your custom indicator level (like Kijun) for R:R calculations:

```json
{
  "indicators": [
    {
      "id": "ichimoku",
      "type": "ichimoku",
      "params": { "tenkan_period": 9, "kijun_period": 26, "senkou_b_period": 52, "displacement": 26 }
    }
  ],
  "risk_settings": {
    "risk_method": "percent",
    "risk_value": 1,
    "rr_ratio": 2.0,
    "spread_buffer_pips": 1,
    "stop_loss_source": {
      "type": "indicator",
      "indicator": "ichimoku",
      "output": "kijun"
    }
  }
}
```

Now:
- Position sizing uses Kijun distance for risk calculation
- `risk_reward_reached` trigger uses Kijun-based R
- TP is placed at 2 × Kijun distance (not 2 × ATR)

#### Example: ATR Multiplier Stop

```json
{
  "risk_settings": {
    "risk_method": "percent",
    "risk_value": 1,
    "rr_ratio": 2.0,
    "spread_buffer_pips": 1,
    "stop_loss_source": {
      "type": "atr_multiplier",
      "multiplier": 1.5
    }
  }
}
```

Stop placed at 1.5 × ATR from entry (parameterizable for optimization).

#### Example: Fixed Pip Stop

```json
{
  "risk_settings": {
    "risk_method": "percent",
    "risk_value": 1,
    "rr_ratio": 2.0,
    "spread_buffer_pips": 1,
    "stop_loss_source": {
      "type": "fixed_pips",
      "pips": 50
    }
  }
}
```

Stop placed at exactly 50 pips from entry.

### Default Stop Loss Calculation (type: 'auto')

When `stop_loss_source` is not specified or set to `{ "type": "auto" }`:

1. **Chandelier exit** (if indicator defined)
2. **ATR-based** (2 × ATR from entry)
3. **2% of price** (fallback)

Take profit is always derived from `rr_ratio × stop_distance`.

---

## Parameters (for Optimization)

Parameters define values that can be tested with different values during backtesting and optimized during walk-forward testing.

**Key concept:** The strategy definition only specifies WHAT parameters exist with their default values. The optimization ranges (min/max/step) are configured in the backtest panel when running Walk-Forward analysis.

```typescript
interface ParameterDefinition {
  id: string;                    // Referenced as { "$param": "id" }
  name: string;
  type: 'number' | 'integer';
  default: number;
  description?: string;          // Optional help text
  group?: 'indicator' | 'entry' | 'exit' | 'risk';
}
```

**Note:** The `min`, `max`, and `step` fields are optional and stored for backwards compatibility, but are not required. Range configuration is now done in the backtest panel when running Walk-Forward.

### Using Parameters

Reference with `{ "$param": "param_id" }` anywhere a number is expected:

**In indicator definitions:**
```json
{
  "indicators": [
    {
      "id": "my_ema",
      "type": "ema",
      "params": { "period": { "$param": "ema_period" } }
    }
  ]
}
```

Then reference in triggers:
```json
{
  "indicator": "my_ema",
  "output": "value"
}
```

**In trigger values:**
```json
{
  "type": "compare",
  "left": { ... },
  "operator": "<",
  "right": { "$param": "rsi_threshold" }
}
```

**In risk settings:**
```json
{
  "rr_ratio": { "$param": "rr_ratio" }
}
```

---

## Common Patterns

### Trend Following with Filter

```json
{
  "indicators": [
    { "id": "fast_ema", "type": "ema", "params": { "period": 9 } },
    { "id": "slow_ema", "type": "ema", "params": { "period": 21 } }
  ],
  "entry_rules": [
    {
      "id": "trend_entry",
      "name": "EMA Cross in Trend",
      "direction": "long",
      "conditions": [
        {
          "primary": {
            "trigger": { "type": "givens", "regime": "trending_up" },
            "negated": false
          },
          "chain": []
        },
        {
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
        }
      ]
    }
  ]
}
```

### S/R Zone Bounce with NOT Overbought

```json
{
  "indicators": [
    { "id": "rsi", "type": "rsi", "params": { "period": 14 } }
  ],
  "entry_rules": [
    {
      "id": "sr_bounce",
      "name": "Bounce off Support (RSI not overbought)",
      "direction": "long",
      "conditions": [
        {
          "primary": {
            "trigger": {
              "type": "compare",
              "left": { "source": "price", "value": "close" },
              "operator": "is_within",
              "right": { "type": "sr_zone", "target": "lower" },
              "distance": { "value": 15, "unit": "pips" }
            },
            "negated": false
          },
          "chain": [
            {
              "operator": "and",
              "trigger": {
                "trigger": {
                  "type": "compare",
                  "left": { "indicator": "rsi", "output": "value" },
                  "operator": ">",
                  "right": { "fixed": 70 }
                },
                "negated": true
              }
            }
          ]
        }
      ]
    }
  ]
}
```

The `negated: true` inverts the RSI > 70 check, so the condition passes when RSI is NOT > 70.

### Pivot Point Take Profit

```json
{
  "exit_rules": [
    {
      "id": "tp_pivot",
      "name": "TP at R1",
      "direction": "long",
      "close_percent": 100,
      "priority": 1,
      "conditions": [{
        "primary": {
          "trigger": {
            "type": "compare",
            "left": { "source": "price", "value": "close" },
            "operator": "is_within",
            "right": { "type": "pivot", "level": "r1" },
            "distance": { "value": 10, "unit": "pips" }
          },
          "negated": false
        },
        "chain": []
      }]
    }
  ]
}
```

### Multiple Exit Priorities

```json
{
  "exit_rules": [
    {
      "id": "sl",
      "name": "Stop Loss",
      "direction": "both",
      "close_percent": 100,
      "priority": 1,
      "conditions": [{ ... }]
    },
    {
      "id": "partial_tp",
      "name": "Take 50% at 1R",
      "direction": "both",
      "close_percent": 50,
      "priority": 2,
      "conditions": [{ ... }]
    },
    {
      "id": "full_tp",
      "name": "Full TP at 2R",
      "direction": "both",
      "close_percent": 100,
      "priority": 3,
      "conditions": [{ ... }]
    }
  ]
}
```

---

## Timeframes

Available timeframes for backtesting and live monitoring:

| Code | Description |
|------|-------------|
| `M1`, `M5`, `M15`, `M30` | Minutes |
| `H1`, `H4` | Hours |
| `D` | Daily |
| `W` | Weekly |

---

## V1 Format (Deprecated)

V1 format is **no longer supported**. All strategies must use V2 format with `conditions` arrays.

If you have old V1 strategies, they must be recreated in V2 format:

- `trigger` field → `conditions` array with `primary` and `chain`
- `weight`/`required` fields → removed (use multiple conditions for AND logic)
- Exit rule `type` → removed (just use triggers)
- Inline indicator references → Separate `indicators` array with ID references

---

## Tips for Creating Strategies

1. **Use conditions for AND logic**: Multiple conditions in a rule are all required (AND)
2. **Use chain for AND/OR within conditions**: Combine triggers with explicit operators
3. **Use negated for NOT logic**: Set `negated: true` to invert any trigger
4. **Define indicators first**: Add to `indicators` array, then reference by ID in triggers
5. **Parameterize**: Make key values optimizable with `{ "$param": "id" }` syntax
6. **Use "is_within"**: For S/R and pivot distance checks
7. **Set exit priorities**: Higher priority exits checked first
8. **Partial exits**: Use `close_percent` < 100 for scaling out
9. **Multi-timeframe**: Add `timeframe` to indicator definitions for higher-timeframe filters (e.g., Daily trend + H1 entry)
