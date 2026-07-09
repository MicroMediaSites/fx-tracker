# Adding New Indicators

This document covers the complete process for adding a new technical indicator to CandleSight. Indicators touch many parts of the system - follow this checklist to ensure full integration.

## Quick Reference

**Files to modify (in order):**

1. `shared/src/lib.rs` - Rust enum
2. `src-tauri/src/backtest/indicators.rs` - Calculation logic
3. `src-tauri/src/backtest/indicator_engine.rs` - Factory function
4. `src/types/strategy.ts` - TypeScript types & metadata (SINGLE SOURCE OF TRUTH)
5. `src/components/charts/chartConstants.ts` - Colors & chart presets

**Files that auto-derive from metadata (no changes needed):**
- `IndicatorEditor.tsx` - Uses `INDICATOR_FULL_NAMES` from metadata
- `DataSourcePicker.tsx` - Uses `ALL_INDICATOR_TYPES` and `INDICATOR_TYPE_LABELS`
- `VariablesSection.tsx` - Uses `getIndicatorsFor('variables')`
- `TriggerChainBuilder.tsx` - Uses `getIndicatorsFor('entryTriggers')`
- `GivensSelector.tsx` - Uses `DIVERGENCE_INDICATOR_TYPES` (derived)
- `indicatorRenderer.ts` - Uses `OVERLAY_INDICATORS` (derived)

---

## Step-by-Step Checklist

### 1. Backend Type Definition

**File:** `shared/src/lib.rs`

Add to the `IndicatorType` enum:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndicatorType {
    Sma,
    Ema,
    // ... existing types
    NewIndicator,  // ADD HERE
}

impl IndicatorType {
    pub fn as_str(&self) -> &'static str {
        match self {
            // ... existing matches
            Self::NewIndicator => "new_indicator",  // ADD HERE
        }
    }
}
```

### 2. Backend Calculation

**File:** `src-tauri/src/backtest/indicators.rs`

Add a new struct implementing the `Indicator` trait:

```rust
// ============================================================================
// New Indicator Name
// ============================================================================

pub struct NewIndicator {
    period: usize,
    // ... other state
}

impl NewIndicator {
    pub fn new(period: usize) -> Self {
        Self {
            period,
            // ... initialize state
        }
    }
}

impl Indicator for NewIndicator {
    fn on_candle(&mut self, candle: &Candle) -> IndicatorOutputs {
        let mut outputs = HashMap::new();

        // Calculate indicator values
        // outputs.insert("value".to_string(), calculated_value);

        outputs
    }

    fn indicator_type(&self) -> &str {
        "new_indicator"
    }

    fn output_names(&self) -> Vec<&str> {
        vec!["value"]  // List all outputs
    }

    fn reset(&mut self) {
        // Clear all state
    }
}
```

### 3. Backend Factory

**File:** `src-tauri/src/backtest/indicator_engine.rs`

Add to the `create_indicator` match:

```rust
fn create_indicator(
    indicator_type: IndicatorType,
    params: &HashMap<String, f64>,
) -> Result<Box<dyn Indicator>, String> {
    match indicator_type {
        // ... existing matches
        IndicatorType::NewIndicator => {
            let period = get_param_usize(params, "period", 14)?;
            Ok(Box::new(NewIndicator::new(period)))
        }
    }
}
```

### 4. Frontend Type Definitions (SINGLE SOURCE OF TRUTH)

**File:** `src/types/strategy.ts`

This is the single source of truth for indicator metadata. Update these locations:

```typescript
// 1. Add to IndicatorType union (~line 115)
export type IndicatorType =
  | 'sma'
  // ... existing types
  | 'new_indicator';

// 2. Add to INDICATOR_METADATA (~line 145)
// This is the SINGLE SOURCE OF TRUTH - all other constants derive from this
export const INDICATOR_METADATA: Record<IndicatorType, IndicatorMetadata> = {
  // ... existing
  new_indicator: {
    label: 'NewInd',              // Short name for compact displays
    fullName: 'New Indicator',    // Full name for detailed views
    isOverlay: false,             // true = draws on price chart, false = separate pane
    // excludeFrom is optional - only add if indicator should be hidden somewhere
    // excludeFrom: ['divergence', 'entryTriggers'],  // Restrict where it appears
  },
};

// 3. Add to INDICATOR_OUTPUTS (~line 180)
export const INDICATOR_OUTPUTS: Record<IndicatorType, string[]> = {
  // ... existing
  new_indicator: ['value', 'signal'],  // List all outputs
};

// 4. Add to OUTPUT_LABELS (~line 199) - only if outputs need friendly names
export const OUTPUT_LABELS: Record<string, string> = {
  // ... existing
  // New Indicator outputs (only add if needed)
  new_output: 'New Output Name',
};

// 5. Add to INDICATOR_DEFAULTS (~line 244)
export const INDICATOR_DEFAULTS: Record<IndicatorType, Record<string, number>> = {
  // ... existing
  new_indicator: { period: 14 },
};
```

**Understanding INDICATOR_METADATA:**

The metadata interface supports these properties:

```typescript
export interface IndicatorMetadata {
  label: string;              // Short name: "RSI", "MACD"
  fullName?: string;          // Long name: "Relative Strength Index"
  isOverlay?: boolean;        // Renders on price chart (SMA, EMA) vs separate pane (RSI, MACD)
  excludeFrom?: IndicatorContext[];  // Contexts to hide this indicator from
}
```

**Available contexts for `excludeFrom`:**

| Context | Description |
|---------|-------------|
| `'overlay'` | Chart overlay rendering |
| `'divergence'` | Divergence indicator selector |
| `'entryTriggers'` | Entry rule indicator selectors |
| `'exitTriggers'` | Exit rule indicator selectors |
| `'variables'` | Variable expression sources |
| `'dataSource'` | DataSourcePicker |

**Example configurations:**

```typescript
// Oscillator that supports divergence (RSI, MACD, Stochastic)
rsi: { label: 'RSI', fullName: 'Relative Strength Index' },  // No excludeFrom = available everywhere

// Overlay indicator (SMA, EMA)
sma: { label: 'SMA', fullName: 'Simple Moving Average', isOverlay: true, excludeFrom: ['divergence'] },

// Exit-only indicator (Chandelier)
chandelier: { label: 'Chandelier', fullName: 'Chandelier Exit', isOverlay: true, excludeFrom: ['divergence', 'entryTriggers', 'variables'] },
```

### 5. Chart Configuration

**File:** `src/components/charts/chartConstants.ts`

Update two locations:

```typescript
// 1. Add to INDICATOR_COLORS (~line 2)
export const INDICATOR_COLORS: Record<string, string> = {
  // ... existing
  new_indicator: '#FF6B6B',
  'new_indicator.signal': '#4ECDC4',  // For multi-output indicators
};

// 2. Add to AVAILABLE_INDICATORS (~line 46) - Chart menu presets
export const AVAILABLE_INDICATORS = {
  // ... existing
  new_indicator: {
    id: 'new_indicator',
    type: 'new_indicator',
    params: { period: 14 },
    label: 'New Indicator (14)',
    category: 'Momentum',  // or Trend, Volatility, Advanced
  },
};
```

**Note:** `OVERLAY_INDICATORS` is now auto-derived from `INDICATOR_METADATA.isOverlay` - no need to update it manually.

---

## Derived Constants (Auto-Updated)

These constants are automatically derived from `INDICATOR_METADATA`:

| Constant | Derived From | Usage |
|----------|--------------|-------|
| `ALL_INDICATOR_TYPES` | `Object.keys(INDICATOR_METADATA)` | All indicator types as array |
| `INDICATOR_TYPE_LABELS` | `meta.label` | Short display names |
| `INDICATOR_FULL_NAMES` | `meta.fullName ?? meta.label` | Full display names |
| `OVERLAY_INDICATOR_TYPES` | `meta.isOverlay === true` | Indicators that render on price chart |
| `DIVERGENCE_INDICATOR_TYPES` | `!meta.excludeFrom?.includes('divergence')` | Divergence-compatible indicators |

Use `getIndicatorsFor(context)` to get indicators available for a specific context:

```typescript
// Get indicators for divergence selector
const divergenceIndicators = getIndicatorsFor('divergence');

// Get indicators for entry triggers
const entryIndicators = getIndicatorsFor('entryTriggers');
```

---

## Testing Checklist

After implementation, verify:

- [ ] Indicator appears in Strategy Builder dropdown
- [ ] Can add indicator to strategy and configure params
- [ ] Can use indicator outputs in entry rules (L and R side)
- [ ] Can use indicator outputs in exit rules (if not excluded)
- [ ] Backtest runs with indicator (check logs for values)
- [ ] Indicator appears in Chart indicator menu
- [ ] Indicator renders correctly on chart (overlay or oscillator pane)
- [ ] Live Monitor evaluates indicator on live candles
- [ ] AI analysis mentions indicator when relevant

---

## Indicator Categories

Use these categories in `AVAILABLE_INDICATORS`:

| Category | Examples |
|----------|----------|
| **Trend** | SMA, EMA, MA Bands, ADX |
| **Momentum** | RSI, MACD, Stochastic, DSS, MA Histogram |
| **Volatility** | ATR, ADR, Bollinger Bands |
| **Advanced** | Ichimoku, Chandelier Exit |

---

## Common Patterns

### Indicators with Multiple Outputs

Examples: MACD (macd, signal, histogram), Bollinger (upper, middle, lower)

- List all outputs in `output_names()` and `INDICATOR_OUTPUTS`
- Add colors for each output in `INDICATOR_COLORS` using dot notation: `'macd.signal': '#color'`
- Add friendly names to `OUTPUT_LABELS` if needed

### Indicators Requiring Daily Data

Example: ADR (Average Daily Range)

- Track daily boundaries (new day detection)
- Aggregate intraday candles to daily
- Calculate on daily close, hold value until next daily close

### Overlay vs Oscillator

- **Overlay** (same scale as price): Set `isOverlay: true` - SMA, EMA, Bollinger, Ichimoku
- **Oscillator** (separate pane): Leave `isOverlay` unset or false - RSI, MACD, Stochastic

### Exit-Only Indicators

For indicators that only make sense for exit rules (like Chandelier Exit):

```typescript
chandelier: {
  label: 'Chandelier',
  fullName: 'Chandelier Exit',
  isOverlay: true,
  excludeFrom: ['divergence', 'entryTriggers', 'variables'],
},
```

### Divergence-Compatible Indicators

Oscillators work best for divergence. For non-oscillators, add `'divergence'` to `excludeFrom`:

```typescript
sma: {
  label: 'SMA',
  excludeFrom: ['divergence'],  // Not an oscillator, doesn't support divergence
},
```
