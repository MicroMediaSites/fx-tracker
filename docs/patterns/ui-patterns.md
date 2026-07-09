# UI Patterns & Design System

This document captures the design philosophy and patterns used across CandleSight windows. Use this as a reference when building or modifying UI components.

---

## Core Design Philosophy: Minimal Chrome

**The most important principle**: CandleSight uses a **minimal chrome** aesthetic where content floats on the page with very little visual container "boxing". This is NOT a traditional dashboard with cards and panels everywhere.

### The Guiding Principle

> Content should feel like it's floating on a dark background, not trapped inside boxes.

Think of it like a Bloomberg terminal or a high-end trading interface - information-dense but visually clean. Every border, background, and container you add is visual noise that competes with the actual data.

### What This Means in Practice

| Aspect | ❌ WRONG (Dashboard-y) | ✅ RIGHT (Minimal Chrome) |
|--------|------------------------|---------------------------|
| **Sections** | Cards with borders and backgrounds | CollapsibleSection with just title text |
| **Lists** | Tables with column headers and grid lines | Rows with left accent border only |
| **Buttons** | Filled buttons (`bg-blue-600`) | Ghost buttons (border only, transparent fill) |
| **Status** | Badges and pills ("● Live", "Short") | Subtle text or single accent color |
| **Grouping** | Explicit card containers | Spacing and typography hierarchy |
| **Controls** | Dropdowns, toggles, switches | Text toggles, cycle selectors |

### Visual Examples

**Section Headers - WRONG:**
```tsx
// ❌ Card-style section with background
<div className="bg-gray-800 border border-gray-700 rounded-lg p-4">
  <h2 className="text-lg font-bold mb-4">Economic Calendar</h2>
  {content}
</div>
```

**Section Headers - RIGHT:**
```tsx
// ✅ Minimal CollapsibleSection - just text
<CollapsibleSection id="calendar" title="Economic Calendar">
  {content}
</CollapsibleSection>
```

**Data Rows - WRONG:**
```tsx
// ❌ Traditional table with headers and borders
<table className="w-full border border-gray-700">
  <thead className="bg-gray-800">
    <tr>
      <th className="border-b border-gray-700 p-2">INSTRUMENT</th>
      <th className="border-b border-gray-700 p-2">DIRECTION</th>
    </tr>
  </thead>
  <tbody>
    <tr className="border-b border-gray-700">
      <td>EUR/USD</td>
      <td><span className="bg-green-600 px-2 rounded">Long</span></td>
    </tr>
  </tbody>
</table>
```

**Data Rows - RIGHT:**
```tsx
// ✅ Borderless rows with left accent only
<div className="flex flex-col">
  <div
    className="flex items-center gap-4 py-2 hover:bg-[var(--color-bg-hover)]"
    style={{ borderLeft: '3px solid var(--color-buy)' }}
  >
    <span className="text-[var(--color-buy)] text-xs font-medium w-12">LONG</span>
    <span className="font-medium">EUR/USD</span>
    <span className="font-mono text-[var(--color-text-secondary)]">1.09234</span>
  </div>
</div>
```

**Buttons - WRONG:**
```tsx
// ❌ Filled buttons everywhere
<button className="bg-blue-600 hover:bg-blue-500 px-4 py-2 rounded">
  Submit
</button>
```

**Buttons - RIGHT:**
```tsx
// ✅ Ghost/outline buttons
<button className="border border-gray-700 hover:border-gray-500 px-4 py-2 text-gray-400 hover:text-gray-200">
  Buy Order
</button>

// ✅ Or for trade actions, subtle filled with low opacity
<button className="bg-[var(--color-buy)]/80 hover:bg-[var(--color-buy)] px-3 py-1 text-xs">
  Order
</button>
```

**Status Indicators - WRONG:**
```tsx
// ❌ Heavy badges and pills
<div className="flex items-center gap-2">
  <span className="bg-green-600 text-white px-2 py-0.5 rounded-full text-xs">● Live</span>
  <span className="bg-red-600 text-white px-2 py-0.5 rounded-full text-xs">Short</span>
</div>
```

**Status Indicators - RIGHT:**
```tsx
// ✅ Subtle text or minimal dots
<span className="text-[10px] text-[var(--color-text-muted)]">
  ⏸ Stop on close
</span>

// ✅ Direction as colored text, not a badge
<span className="text-[var(--color-sell)] text-xs font-medium">SHORT</span>
```

**Toggle Controls - WRONG:**
```tsx
// ❌ iOS-style toggle switches
<Toggle checked={enabled} onChange={setEnabled} />
<span className="ml-2">Keep running</span>
```

**Toggle Controls - RIGHT:**
```tsx
// ✅ Text toggle that changes on click
<button onClick={() => setEnabled(!enabled)} className="text-[10px]">
  {enabled ? '▶ Run in background' : '⏸ Stop on close'}
</button>
```

### Anti-Patterns to Avoid

1. **Over-boxing**: Don't put borders around every section. Use spacing instead.

2. **Status badge overload**: Don't add "● Live", "● Streaming", "Active" badges everywhere. If something is live by default, you don't need to say it.

3. **Column headers in simple lists**: If the data is self-evident (instrument, price, direction), you don't need headers. The Pattern Matches list in Live Monitor has no column headers.

4. **Card grids**: Don't create side-by-side cards with borders for related content. Let content flow naturally.

5. **Filled buttons for non-primary actions**: Reserve filled buttons for primary trade actions (Buy/Sell). Everything else should be ghost/outline or just text.

6. **Explicit dividers**: Don't add `<hr>` or border-bottom everywhere. Use vertical spacing (`space-y-*`) to separate content.

### When Containers ARE Appropriate

Containers (subtle borders/backgrounds) are appropriate for:

1. **Interactive cards** like the price window in FX Ticket - these have clear boundaries because you interact with them
2. **Form groups** where inputs need visual grouping
3. **Modal dialogs** - these need clear boundaries
4. **Drag targets** or drop zones

Even then, keep borders subtle (`border-gray-700` not `border-gray-500`) and avoid heavy backgrounds.

### The Litmus Test

Before adding a container, border, or background, ask:
1. Does removing this make the content harder to understand?
2. Is this grouping already clear from spacing and typography?
3. Would a professional trading interface have this chrome?

If the answer to #1 is "no" or #2 is "yes" or #3 is "no" - don't add it.

### Summary Metrics / KPI Display

For account balances, stats, or KPIs, don't use bordered cards:

```tsx
// ❌ Wrong - card container for metrics
<div className="border border-gray-700 rounded-lg p-4">
  <div className="grid grid-cols-4 gap-4">
    <div>
      <span className="text-xs text-gray-500">BALANCE</span>
      <span className="text-xl font-bold">$10,107.80</span>
    </div>
    ...
  </div>
</div>

// ✅ Right - floating metrics with typography hierarchy
<div className="flex items-baseline gap-8 py-4">
  <div>
    <span className="text-[10px] text-[var(--color-text-muted)] uppercase tracking-wide">Balance</span>
    <div className="text-xl font-mono text-[var(--color-text-primary)]">$10,107.80</div>
  </div>
  <div>
    <span className="text-[10px] text-[var(--color-text-muted)] uppercase tracking-wide">Unrealized P/L</span>
    <div className="text-xl font-mono text-[var(--color-buy)]">+$42.50</div>
  </div>
  ...
</div>
```

### Side-by-Side Sections

Don't create parallel card containers:

```tsx
// ❌ Wrong - side-by-side bordered cards
<div className="grid grid-cols-2 gap-4">
  <div className="border border-gray-700 rounded-lg p-4">
    <h3>Open Positions (0)</h3>
    <p>No open positions</p>
  </div>
  <div className="border border-gray-700 rounded-lg p-4">
    <h3>Pending Orders (0)</h3>
    <p>No pending orders</p>
  </div>
</div>

// ✅ Right - use CollapsibleSections stacked, or inline content
<CollapsibleSection id="positions" title="Open Positions" badge="(0)">
  <p className="text-[var(--color-text-muted)] text-sm py-4">No open positions</p>
</CollapsibleSection>

<CollapsibleSection id="orders" title="Pending Orders" badge="(0)">
  <p className="text-[var(--color-text-muted)] text-sm py-4">No pending orders</p>
</CollapsibleSection>
```

If you need side-by-side, use the same minimal styling as stacked:

```tsx
// ✅ Acceptable - side-by-side but still minimal
<div className="grid grid-cols-2 gap-8">
  <div>
    <h3 className="text-sm font-medium mb-2">Open Positions</h3>
    <p className="text-[var(--color-text-muted)] text-sm">No open positions</p>
  </div>
  <div>
    <h3 className="text-sm font-medium mb-2">Pending Orders</h3>
    <p className="text-[var(--color-text-muted)] text-sm">No pending orders</p>
  </div>
</div>
```

---

## Color System

We use CSS custom properties for theming. The system supports both dark and light modes.

### Semantic Colors - Use These

| Variable | Purpose | Example Usage |
|----------|---------|---------------|
| `--color-buy` | Long/buy actions | Buy buttons, long position indicators |
| `--color-buy-text` | Text on buy backgrounds | Button text |
| `--color-sell` | Short/sell actions | Sell buttons, short position indicators |
| `--color-sell-text` | Text on sell backgrounds | Button text |
| `--color-warning` | Caution/exit signals | Exit match indicators, warnings |
| `--color-warning-text` | Text for warnings | Warning labels |
| `--color-info` | Informational/neutral | Strategy names, metadata |
| `--color-info-text` | Informational text | Links, highlights |

### Text Hierarchy

| Variable | Purpose |
|----------|---------|
| `--color-text-primary` | Main content, headings |
| `--color-text-secondary` | Secondary content, values |
| `--color-text-muted` | Labels, timestamps, less important info |
| `--color-text-faint` | Disabled states, placeholders |

### Backgrounds

| Variable | Purpose |
|----------|---------|
| `--color-bg-page` | Main page background |
| `--color-bg-elevated` | Elevated surfaces |
| `--color-bg-card` | Card backgrounds |
| `--color-bg-hover` | Hover states |

### DO NOT USE

- `--color-success` / `--color-error` - These don't exist. Use `--color-buy` / `--color-sell` instead.

### Color Opacity Patterns

```tsx
// Faded backgrounds
bg-[var(--color-buy)]/20      // 20% opacity for subtle backgrounds
bg-[var(--color-sell)]/80     // 80% opacity for buttons

// Faded text (for confirmations, etc.)
text-[var(--color-buy)]/60    // 60% opacity for faded confirmation text
```

## Trade Confirmation UX

When a trade executes from a signal row:

1. **Inline confirmation** - Replace the Order button with confirmation text
2. **Fade the row** - Set row to ~60% opacity
3. **Keep dismiss available** - X button stays visible
4. **No auto-removal** - Row stays until user dismisses or expiry removes it

```tsx
// Confirmation text (faded trade color)
<span className={`text-[10px] font-medium text-[var(--color-buy)]/60`}>
  BOUGHT 10,000 @ 1.08123
</span>

// Row fading
className={`... ${confirmation ? 'opacity-60' : 'hover:bg-...'}`}
```

### Why Not Overlays?

- Overlays are intrusive when monitoring multiple signals
- Inline confirmation keeps context visible
- User maintains control over when to dismiss

## Button Patterns

### Button Hierarchy

CandleSight uses a strict button hierarchy to maintain the minimal chrome aesthetic:

| Type | Usage | Style |
|------|-------|-------|
| **Ghost** | Most actions (navigation, settings, secondary) | Border only, transparent fill |
| **Trade Action** | Buy/Sell/Order/Close | Subtle filled with trade color |
| **Text** | Toggles, inline actions | No border, just text that changes |
| **Danger** | Destructive actions | Red text or border, never filled red |

### Ghost Buttons (Default)

Use ghost buttons for most interactive elements:

```tsx
// Standard ghost button
<button className="border border-[var(--color-border)] hover:border-[var(--color-text-muted)]
  px-4 py-2 text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)]
  transition-colors">
  Buy Order
</button>

// Small ghost button
<button className="border border-[var(--color-border)] hover:border-[var(--color-text-muted)]
  px-2 py-1 text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)]">
  Settings
</button>
```

### Trade Execution Buttons

- Use "Order" not "Trade" for entry signals
- Use "Close" for exit signals
- Fixed width (`w-14`) to prevent layout shift between states
- These are the ONLY buttons that should be filled (and even then, with opacity)

### Stale Trade Two-Step Confirmation

When a signal goes stale, require confirmation:

1. First click: Button shows "Stale" → changes to "Confirm"
2. Second click: Executes the trade

```tsx
const [staleConfirmed, setStaleConfirmed] = useState(false);

const handleTradeClick = () => {
  if (isStale && !staleConfirmed) {
    setStaleConfirmed(true);
    return;
  }
  handleExecute();
};
```

### Button Color Patterns

```tsx
// Buy/Long button
bg-[var(--color-buy)]/80 hover:bg-[var(--color-buy)]

// Sell/Short button
bg-[var(--color-sell)]/80 hover:bg-[var(--color-sell)]

// Stale/muted button
bg-[var(--color-text-muted)]/30 hover:bg-[var(--color-text-muted)]/50
```

### Text Toggles

For binary options, prefer text toggles over iOS-style switches:

```tsx
// Text toggle - changes label and icon based on state
<button
  onClick={() => setEnabled(!enabled)}
  className={`text-[10px] transition-colors ${
    enabled
      ? 'text-[var(--color-buy)] hover:text-[var(--color-buy-text)]'
      : 'text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)]'
  }`}
>
  {enabled ? '▶ Run in background' : '⏸ Stop on close'}
</button>
```

Benefits:
- Takes less space than toggle + label
- The label itself describes the current state
- Fits the minimal chrome aesthetic
- Works well at small sizes (`text-[10px]`)

## Typography-Based Hierarchy

Use typography and spacing to create visual hierarchy instead of borders/containers:

### Text Sizes

| Size | Usage |
|------|-------|
| `text-lg` / `text-xl` | Window titles only |
| `text-sm` / `text-base` | Section titles, primary content |
| `text-xs` | Secondary content, labels |
| `text-[10px]` | Timestamps, metadata, toggles |

### Font Weights

| Weight | Usage |
|--------|-------|
| `font-semibold` | Section titles |
| `font-medium` | Emphasized content (instruments, prices) |
| `font-normal` | Body text |
| — | Use `font-mono` for all numbers/prices |

### Creating Groups Without Borders

```tsx
// ❌ Wrong - using a card container
<div className="border border-gray-700 rounded p-4 mb-4">
  <h3>Account Info</h3>
  <p>Balance: $10,000</p>
</div>

// ✅ Right - using spacing and typography
<div className="mb-6">
  <h3 className="text-sm font-medium text-[var(--color-text-primary)] mb-2">
    Account Info
  </h3>
  <p className="text-[var(--color-text-secondary)]">
    Balance: <span className="font-mono">$10,000</span>
  </p>
</div>
```

### Spacing Guidelines

- Between major sections: `space-y-6` or `mb-6`
- Between items in a list: `space-y-2` or `gap-2`
- Within a data row: `gap-3` or `gap-4`
- Padding inside containers (when needed): `p-4` or `p-5`

## Data Display Patterns

### Lists vs Tables

**Prefer flat row lists over traditional tables.** Tables with headers and grid lines feel heavy and "dashboard-y".

```tsx
// ❌ Traditional table - too heavy
<table className="w-full">
  <thead>
    <tr className="border-b border-gray-700 text-left text-xs text-gray-500">
      <th className="p-2">INSTRUMENT</th>
      <th className="p-2">DIRECTION</th>
      <th className="p-2">PRICE</th>
    </tr>
  </thead>
  <tbody>
    {items.map(item => (
      <tr className="border-b border-gray-800">
        <td className="p-2">{item.instrument}</td>
        ...
      </tr>
    ))}
  </tbody>
</table>

// ✅ Flat row list - minimal and clean
<div className="flex flex-col">
  {items.map(item => (
    <div
      key={item.id}
      className="flex items-center gap-4 py-2 px-1 hover:bg-[var(--color-bg-hover)]"
      style={{ borderLeft: `3px solid var(--color-${item.direction === 'long' ? 'buy' : 'sell'})` }}
    >
      <span className={`text-xs font-medium w-12 ${
        item.direction === 'long' ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'
      }`}>
        {item.direction.toUpperCase()}
      </span>
      <span className="font-medium">{item.instrument}</span>
      <span className="font-mono text-[var(--color-text-secondary)]">{item.price}</span>
    </div>
  ))}
</div>
```

**When to use column headers:**
- Complex data with 6+ columns where meaning isn't obvious
- Sortable columns (indicate sort capability)
- Data export/comparison scenarios

**When to skip column headers:**
- Simple lists (instrument, direction, price)
- Pattern matches, trade signals
- Most real-time data feeds

### Row Patterns

```tsx
// Standard data row with left accent
<div
  className="flex items-center gap-4 py-2 hover:bg-[var(--color-bg-hover)] transition-colors"
  style={{ borderLeft: '3px solid var(--color-buy)' }}
>
  {/* Content */}
</div>

// Grid row for aligned columns
<div
  className="grid items-center gap-x-2 py-2 hover:bg-[var(--color-bg-hover)]"
  style={{
    gridTemplateColumns: '48px 80px 100px 1fr auto',
    borderLeft: '3px solid var(--color-buy)'
  }}
>
  {/* Content */}
</div>

// Expandable row (like monitors in Live Monitor)
<div className="border-b border-[var(--color-border)]/30">
  <button className="w-full flex items-center gap-2 py-2 hover:bg-[var(--color-bg-hover)]">
    <ChevronIcon className={expanded ? 'rotate-90' : ''} />
    <span>{title}</span>
  </button>
  {expanded && <div className="pl-6 pb-2">{children}</div>}
</div>
```

### Price Display

- Use `font-mono` for all prices
- Big figures emphasized, pipette smaller
- Include spread indicator where relevant

### Stop Loss / Take Profit

- Use **neutral colors** (`text-secondary`), not red/green
- Red/green implies bid/ask which is confusing
- Use clear labels: "SL" and "TP" (not just "S" and "T")

```tsx
// Good - neutral with clear labels
<span className="text-[var(--color-text-secondary)]">
  <span className="text-[var(--color-text-muted)]">SL</span> 1.08000
</span>

// Bad - colored like bid/ask
<span className="text-[var(--color-sell)]">S 1.08000</span>
```

### Direction Indicators

Use left border color to indicate trade direction:

```tsx
// Long entry - green
style={{ borderLeftColor: 'var(--color-buy)' }}

// Short entry - red
style={{ borderLeftColor: 'var(--color-sell)' }}

// Exit signal - orange/warning
style={{ borderLeftColor: 'var(--color-warning)' }}

// Stale - muted
style={{ borderLeftColor: 'var(--color-text-muted)' }}
```

## Layout Components

### CollapsibleSection

Reusable collapsible panel with:
- Persistent collapse state (localStorage)
- Optional resizable height with drag handle
- Badge and action slots in header

```tsx
<CollapsibleSection
  id="monitors"
  title="Active Monitors"
  badge={<span className="...">{count}</span>}
  action={<button>+</button>}
  resizable
  defaultHeight={300}
  minHeight={150}
  maxHeight={600}
>
  {children}
</CollapsibleSection>
```

### Grid Layouts for Data Rows

Use CSS Grid for consistent column alignment:

```tsx
<div
  className="grid items-center gap-x-2"
  style={{ gridTemplateColumns: '56px 72px 72px 80px 80px 1fr 32px 36px 56px 20px auto' }}
>
```

### Responsive Layouts

- Wide view: Single-row grid layout
- Narrow view: Stacked/wrapped layout
- Use `hidden md:grid` and `md:hidden` to switch

## Performance Patterns

### Render Isolation for Prices

Subscribe to prices at the leaf component level to prevent parent re-renders:

```tsx
// Good - isolated subscription
function MidPrice({ instrument }: { instrument: string }) {
  const price = usePriceStore((state) => state.prices[instrument]);
  // Only this component re-renders on price change
}

// Bad - parent subscribes and passes down
function ParentCard({ instrument }) {
  const price = usePriceStore((state) => state.prices[instrument]);
  return <MidPrice price={price} />; // Parent re-renders too
}
```

### Zustand Selective Subscriptions

Always use selectors to subscribe to specific slices:

```tsx
// Good
const price = usePriceStore((state) => state.prices[instrument]);

// Bad - subscribes to entire store
const { prices } = usePriceStore();
```

## Animation Patterns

### Fade Out + Collapse (for expiring rows)

```tsx
const [animationPhase, setAnimationPhase] = useState<'none' | 'fading' | 'collapsing'>('none');

// Trigger sequence
setAnimationPhase('fading');
setTimeout(() => setAnimationPhase('collapsing'), 1300);
setTimeout(() => onRemove(), 1800);

// CSS classes
const getAnimationClasses = () => {
  switch (animationPhase) {
    case 'fading':
      return 'opacity-0 scale-95 max-h-24';
    case 'collapsing':
      return 'opacity-0 scale-95 max-h-0 py-0 overflow-hidden';
    default:
      return 'opacity-100 max-h-24';
  }
};
```

### Status Indicators

Pulsing dot for active/running states:

```tsx
<span className={`w-2 h-2 rounded-full ${
  status === 'running'
    ? 'bg-[var(--color-buy)] animate-pulse'
    : status === 'error'
    ? 'bg-[var(--color-sell)]'
    : 'bg-[var(--color-text-muted)]'
}`} />
```

## Input Patterns

### Threshold Inputs (stale/expire config)

```tsx
const INPUT_CLASS = 'w-9 px-1 py-0.5 text-[10px] bg-transparent border border-[var(--color-border)] rounded text-center text-[var(--color-text-primary)] focus:outline-none focus:border-[var(--color-info)]';
```

- Commit on blur or Enter
- Select all on focus
- Filter non-numeric input

### CycleSelector

Click-to-cycle through options (better than dropdowns for small option sets):

```tsx
<CycleSelector
  options={['pips', 'percent_sl', 'spread', 'minutes']}
  value={unit}
  onChange={setUnit}
  formatLabel={(u) => LABELS[u]}
/>
```

## Window Header

All windows use `WindowHeader` component with:
- Title
- Settings toggle
- AI terminal integration
- Window navigation

```tsx
<WindowHeader
  title="Strategy Watcher"
  currentWindow="watcher"
  settingsOpen={settingsOpen}
  onSettingsChange={setSettingsOpen}
  terminalContextProvider={() => buildContext(...)}
/>
```

## FX Price Display Conventions

### Price Parts Formatting

FX prices are displayed in three parts for readability:

```
Standard pair (EUR/USD): 1.09234
  top: "1.09"    (big figure)
  big: "23"      (pips - emphasized)
  small: "4"     (pipette)

JPY pair (USD/JPY): 109.234
  top: "109."    (big figure)
  big: "23"      (pips - emphasized)
  small: "4"     (pipette)
```

Use `formatPriceParts()` from `lib/priceCalculations.ts`:

```tsx
const parts = formatPriceParts(price, isJpy);
<span className="text-[10px] text-muted">{parts.top}</span>
<span className="text-2xl font-semibold">{parts.big}</span>
<span className="text-xs">{parts.small}</span>
```

### JPY Pair Handling

JPY pairs use different decimal precision:

| | Standard | JPY |
|---|---|---|
| Decimals | 5 | 3 |
| Pip multiplier | 10,000 | 100 |
| Example | 1.09234 | 109.234 |

Detection: `instrument.includes('JPY')`

### Bid/Ask Layout Convention

Standard FX convention - match what traders expect:

```
┌─────────────┬─────────────┐
│   SELL      │     BUY     │
│   (Bid)     │    (Ask)    │
│   Left      │    Right    │
└─────────────┴─────────────┘
```

The PriceWindow component follows this with:
- Left side: Sell at bid price
- Right side: Buy at ask price
- Center notch: Spread indicator

### Price Flash Animation

Use `usePriceFlash` hook to flash colors on price changes:

```tsx
import { usePriceFlash } from '../hooks/usePriceFlash';

const bidFlash = usePriceFlash(price?.bid);

// Apply color based on direction
const getFlashColor = (flash: PriceDirection | null) => {
  if (flash === 'up') return 'text-[var(--color-buy-text)]';
  if (flash === 'down') return 'text-[var(--color-sell-text)]';
  return 'text-[var(--color-text-primary)]';
};

<span className={`transition-colors duration-300 ${getFlashColor(bidFlash)}`}>
  {parts.big}
</span>
```

The hook:
- Compares string prices (avoids floating point issues)
- Returns 'up', 'down', or 'neutral'
- Auto-resets to neutral after flash duration (default 500ms)

### Spread Visualization

Spread bar uses logarithmic scale to handle both tight and wide spreads:

```tsx
// 0 pips = 0%, ~1 pip = 40%, ~5 pips = 70%, ~20 pips = 90%, 50+ = 100%
const barWidthPercent = spreadNum <= 0
  ? 0
  : Math.min(100, (Math.log10(spreadNum + 1) / Math.log10(51)) * 100);
```

### MidPrice Component

For compact price display (e.g., in cards), use `MidPrice`:

```tsx
<MidPrice instrument="EUR_USD" />
```

Features:
- Subscribes to price internally (render isolation)
- Shows mid price: `(bid + ask) / 2`
- Pipette lifted as superscript

### Key Files

- `src/lib/priceCalculations.ts` - Price formatting, pip calculations
- `src/hooks/usePriceFlash.ts` - Price change flash animation
- `src/components/ui/PriceDisplay.tsx` - PriceWindow and MidPrice components
- `src/stores/priceStore.ts` - Zustand store for live prices

## Common Gotchas

1. **CSS variables in Tailwind** - Use bracket syntax: `bg-[var(--color-buy)]`

2. **Dynamic border colors** - Tailwind can't process dynamic values. Use inline styles:
   ```tsx
   // Won't work
   className={`border-l-[var(--color-${direction})]`}

   // Works
   style={{ borderLeftColor: getBorderColor() }}
   ```

3. **Opacity with CSS variables** - Append opacity: `bg-[var(--color-buy)]/20`

4. **Status updates removing cards** - If updating status removes a card from a list, delay the update to allow confirmation UI to show first.

5. **Font for prices** - Always use `font-mono` for prices/numbers for alignment.

## Strategy Builder & Form Panel Patterns

The Strategy Builder and similar configuration panels follow the same minimal chrome philosophy. Here's how to apply it:

### Collapsible Sections in Forms

Use simple collapsible sections with thin separators, not heavy containers:

```tsx
// ❌ Wrong - heavy container around collapsible content
<div className="bg-[var(--color-bg-elevated)]/50 rounded-lg border border-[var(--color-border)]">
  <button className="w-full px-4 py-3 flex items-center ...">
    <span className="font-medium">{title}</span>
  </button>
  {expanded && <div className="px-4 pb-4">{children}</div>}
</div>

// ✅ Right - minimal chrome with just a border separator
<div>
  <button className="w-full py-2 flex items-center border-b border-[var(--color-border)]">
    <svg className="w-4 h-4 ..." />
    <span className="font-medium">{title}</span>
  </button>
  {expanded && <div className="pt-4">{children}</div>}
</div>
```

### Sub-Section Headers

Use uppercase text headers, not container wrappers:

```tsx
// ❌ Wrong - nested container with h3
<div className="p-4 bg-[var(--color-bg-elevated)] rounded">
  <h3 className="text-md font-medium mb-4">Risk Per Trade</h3>
  {content}
</div>

// ✅ Right - uppercase section header
<div>
  <h4 className="text-xs font-medium text-[var(--color-text-muted)] uppercase tracking-wider mb-3">
    Risk Per Trade
  </h4>
  {content}
</div>
```

### Parameter/Item Cards

Use left border accent instead of full card containers:

```tsx
// ❌ Wrong - heavy card styling
<div className="p-3 bg-[var(--color-bg-elevated)] rounded border border-[var(--color-border)]">
  <span className="font-medium">{param.name}</span>
  <span className="px-1.5 py-0.5 text-xs rounded border bg-[var(--color-buy)]/20">
    {category}
  </span>
</div>

// ✅ Right - left border accent
<div
  className="py-2 pl-3 hover:bg-[var(--color-bg-hover)]"
  style={{ borderLeft: '3px solid var(--color-buy)' }}
>
  <span className="font-medium">{param.name}</span>
  <span className="text-xs text-[var(--color-text-muted)]">{category}</span>
</div>
```

### Empty States

Keep empty states simple - no heavy containers:

```tsx
// ❌ Wrong - heavy bordered empty state
<div className="p-8 bg-[var(--color-bg-elevated)]/50 rounded border border-dashed border-[var(--color-border)] text-center">
  <svg className="w-10 h-10 text-[var(--color-text-muted)] mx-auto mb-2" ... />
  <p className="text-[var(--color-text-muted)]">No items yet.</p>
  <button className="mt-4 ...">Add First Item</button>
</div>

// ✅ Right - simple text empty state
<div className="py-6 text-center">
  <p className="text-[var(--color-text-muted)]">No items yet.</p>
  <p className="text-xs text-[var(--color-text-muted)] mt-1">
    Add items by configuring the form above.
  </p>
  <button className="mt-4 ...">Add First Item</button>
</div>
```

### Progress Bars

Keep progress bars minimal:

```tsx
// ❌ Wrong - thick with heavy track
<div className="w-full bg-[var(--color-bg-elevated)] rounded-full h-2">
  <div className="bg-[var(--color-info)] h-2 rounded-full" style={{ width: `${percent}%` }} />
</div>

// ✅ Right - thin with subtle track
<div className="w-full bg-[var(--color-border)] rounded-full h-1">
  <div className="bg-[var(--color-info)] h-1 rounded-full transition-all" style={{ width: `${percent}%` }} />
</div>
```

### Results Tables

Use left border accent for highlighted rows, not background tinting:

```tsx
// ❌ Wrong - background tint for best result
<tr className={`${i === 0 ? 'bg-[var(--color-buy)]/10' : ''} hover:bg-[var(--color-bg-hover)]`}>
  ...
</tr>

// ✅ Right - left border accent
<tr
  className="hover:bg-[var(--color-bg-hover)]"
  style={i === 0 ? { borderLeft: '3px solid var(--color-buy)' } : undefined}
>
  ...
</tr>
```

### Info/Tip Messages

Replace heavy info boxes with inline tips:

```tsx
// ❌ Wrong - container with border
<div className="p-3 bg-[var(--color-info)]/10 border border-[var(--color-info)]/30 rounded text-sm">
  <strong>Tip:</strong> This is helpful information.
</div>

// ✅ Right - inline text tip
<p className="text-xs text-[var(--color-text-muted)]">
  <span className="text-[var(--color-info-text)]">Tip:</span> This is helpful information.
</p>
```

### Form Field Grouping

For form inputs, use spacing not containers:

```tsx
// ❌ Wrong - nested container groups
<div className="p-3 bg-[var(--color-bg-card)]/50 rounded">
  <div className="grid grid-cols-2 gap-4">
    <div><label>Field 1</label><input /></div>
    <div><label>Field 2</label><input /></div>
  </div>
</div>

// ✅ Right - just the grid, no wrapper
<div className="grid grid-cols-2 gap-4 mt-3">
  <div><label>Field 1</label><input /></div>
  <div><label>Field 2</label><input /></div>
</div>
```

### The Strategy Builder Litmus Test

Before adding a container to any form panel:
1. **Would removing it make the grouping unclear?** - Usually no, spacing handles it
2. **Is this a distinct interactive element?** - Only dropdowns/inputs need backgrounds
3. **Does the shared CollapsibleSection already handle this?** - Let the parent do the work
