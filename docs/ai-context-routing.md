# AI Context Routing Design

## Overview

To reduce token usage (~14k per message), we implement a two-layer context system:
1. **Layer 1 (Base)**: Minimal window state - always sent
2. **Layer 2 (Classified)**: Additional context loaded based on question classification

Classification is performed by GPT-4o-mini before the main Opus call.

---

## Classification Categories

| Category | Intent | Example Questions |
|----------|--------|-------------------|
| `backtest` | Analyzing performance results | "Why is my win rate so low?", "What's causing the drawdown?", "Is 1.2 Sharpe good?", "Why did it underperform in Q3?" |
| `strategy` | Building or modifying strategies | "Add an RSI exit rule", "How do I add a trailing stop?", "Make it exit on divergence", "What indicators can I use?" |
| `trade` | Analyzing a specific trade | "What went wrong here?", "Was my entry timing good?", "Why did I get stopped out?" |
| `market` | Current market data (tool-heavy) | "What's EUR/USD at?", "What's the spread on gold?", "Show me recent candles" |
| `general` | Greetings, app help, off-topic | "Hello", "Thanks!", "How do I use labels?", "Where are my settings?" |

### Classification Edge Cases

**Multi-category questions:**
- "Why did my RSI strategy lose money and how do I fix it?" → `backtest` + `strategy`
- "Should I take this trade based on my strategy?" → `trade` + `strategy`

**Resolution:** For multi-category, load the union of required contexts. Classifier should return primary + secondary category.

---

## Layer 1: Base Window State

Always sent regardless of classification. Provides orientation.

### Ticket Window
```typescript
{
  window: "ticket",
  instrument: "EUR_USD",
  direction: "long" | "short" | null,
  units: "10000",
  stopLoss: "1.0850",
  takeProfit: "1.0950",
  orderType: "market" | "limit" | "stop",
  currentPrice: { bid: "1.0900", ask: "1.0902" }
}
```

### Chart Window
```typescript
{
  window: "chart",
  instrument: "EUR_USD",
  timeframe: "H4",
  selectedIndicator: "RSI" | null,
  strategyOverlay: "RSI Reversal" | null,
  visibleRange: { start: "2024-01-01", end: "2024-01-15" }
}
```

### Analysis Window
```typescript
{
  window: "analysis",
  dateRange: { start: "2024-01-01", end: "2024-12-31" },
  filtersActive: true,
  activeFilters: { instruments: ["EUR_USD"], direction: "long" },
  activeBreakdown: "session" | "day" | "hour" | "instrument",
  selectedTradeId: "trade_123" | null,
  summaryStats: {
    tradeCount: 150,
    winRate: "62%",
    profitFactor: "1.8"
  }
}
```

### Backtesting Window
```typescript
{
  window: "backtesting",
  strategyId: "strat_abc",
  strategyName: "RSI Reversal",
  methodology: "walkforward" | "in-browser",
  hasResults: true,
  currentParams: {
    rsiPeriod: 14,
    overbought: 70,
    oversold: 30
  }
}
```

### Watcher Window
```typescript
{
  window: "watcher",
  runningStrategies: ["RSI Reversal", "MACD Cross"],
  pendingSignalCount: 3,
  instruments: ["EUR_USD", "GBP_USD", "USD_JPY"]
}
```

### Account Window
```typescript
{
  window: "account",
  environment: "practice" | "live",
  balance: "10000.00",
  unrealizedPL: "-50.00",
  openPositionCount: 2
}
```

---

## Layer 2: Classification-Driven Context

Additional context loaded based on classification category.

### `backtest` Category

**Purpose:** User is analyzing why a strategy performed the way it did.

**Load:**
| Data | Source | Why Needed |
|------|--------|------------|
| Full strategy JSON | Database | Needs to see exact rules that were tested |
| Current backtest results | Database/State | Metrics being discussed |
| Previous 10 backtest runs | Database | For comparison, trend analysis |
| Indicator definitions | Static | Explain what indicators do |
| Sample trades (5 winners, 5 losers) | Database | Concrete examples of what happened |
| Equity curve summary | Database | Overall performance shape |

**Skip:**
- Strategy building documentation
- App usage instructions
- Candle history (unless specifically about price action)
- Other strategies' data

**Estimated tokens:** ~3-5k

---

### `strategy` Category

**Purpose:** User wants to build or modify a strategy.

**Load:**
| Data | Source | Why Needed |
|------|--------|------------|
| Strategy builder documentation | Static | How to express rules |
| Supported indicators list | Static | What's available |
| Indicator parameters & defaults | Static | How to configure them |
| Supported rule types | Static | Entry/exit condition options |
| Current strategy structure | Database | What exists to modify |
| Feasibility constraints | Static | What combinations work |

**Skip:**
- Backtest results
- Historical candle data
- Trade history
- Performance metrics
- Other strategies

**Estimated tokens:** ~2-3k

---

### `trade` Category

**Purpose:** User is analyzing a specific trade.

**Load:**
| Data | Source | Why Needed |
|------|--------|------------|
| Full trade metrics | State/Database | MAE, MFE, efficiency, timing |
| 20 candles before entry | OANDA API | Pre-trade context |
| 20 candles after exit | OANDA API | Post-trade movement |
| All indicator values at entry | Calculated | What indicators showed |
| All indicator values at exit | Calculated | How indicators changed |
| Market context (trend, volatility) | Calculated | Broader picture |

**Skip:**
- Strategy building docs
- Other trades
- Backtest results
- Account summary

**Estimated tokens:** ~2-4k

---

### `market` Category

**Purpose:** User wants current market data. Mostly tool-driven.

**Load:**
| Data | Source | Why Needed |
|------|--------|------------|
| (Nothing extra) | - | Tools will fetch what's needed |

**Skip:**
- Everything except base context

**Note:** This category relies on AI tools (get_current_price, get_recent_candles) rather than pre-loaded context.

**Estimated tokens:** ~0.5k (just base)

---

### `general` Category

**Purpose:** Greetings, thanks, app help questions.

**Load:**
| Data | Source | Why Needed |
|------|--------|------------|
| (Nothing extra) | - | - |

**Skip:**
- Everything except base context

**Estimated tokens:** ~0.5k (just base)

---

## Classifier Prompt

```
You classify user prompts to load the right context for a forex trading AI assistant.

Categories:
- backtest: Analyzing strategy performance - metrics, results, why it performed a certain way
- strategy: Building or modifying strategies - adding rules, indicators, conditions
- trade: Analyzing a specific trade - entry/exit quality, what went wrong, timing
- market: Current market data - prices, spreads, recent candles (will use tools)
- general: Greetings, app help, off-topic, doesn't fit above

Current window: {window_type}
User is viewing: {base_context_summary}

Examples:
"Why is my Sharpe ratio only 0.8?" → backtest
"Add an exit when RSI crosses 70" → strategy
"What went wrong on this trade?" → trade
"What's EUR/USD trading at?" → market
"How do I use labels?" → general
"Thanks!" → general

For questions spanning multiple categories, return primary and secondary.

User prompt: "{prompt}"

Respond with JSON:
{"primary": "<category>", "secondary": "<category or null>", "confidence": "<high|medium|low>"}
```

---

## Implementation Notes

### Fallback Behavior
- If classification fails → default to loading full context (current behavior)
- If confidence is "low" → consider loading union of likely categories

### Caching
- Static data (indicator docs, strategy builder docs) can be cached
- Backtest results can be cached per strategy version

### Token Budget
- Target: < 5k tokens for context (down from ~14k)
- Base context: ~0.5-1k
- Category context: ~2-4k max

---

## Open Questions

1. **Should we cache classification results?** Same question in same window = same category?

2. **How do we handle follow-up questions?** "Why?" after a backtest question should stay in backtest category.

3. **Do we need window-specific category restrictions?** E.g., `trade` category only valid in Analysis window?

4. **What about the `/help` command?** Currently uses Haiku with user guide. Should it use this system?

5. **How do we measure success?** Track tokens per category, user satisfaction?

---

## Migration Plan

1. [ ] Finalize this spec
2. [ ] Update classifier prompt in `queries-service/src/classifier.ts`
3. [ ] Create context loaders for each category
4. [ ] Update `chatContextBuilder.ts` to support layered loading
5. [ ] Wire up in `TerminalOverlay.tsx`
6. [ ] Test with real prompts, measure token reduction
7. [ ] Monitor for quality degradation
