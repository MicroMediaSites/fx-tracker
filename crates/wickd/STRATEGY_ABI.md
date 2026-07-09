# Strategy Script ABI

**Version: 5** (2026-07 · AGT-651; v4: 2026-07 · AGT-632; v3: 2026-07 · #290; v2: 2026-07 · #289; v1: 2026-06 · AGT-606, AGT-609)

This is the versioned contract for `.rhai` strategy scripts consumed by
`wickd strategy run`, `wickd backtest`, and the live watcher daemon
(`wickd watch` / `StrategyWatcher` / `MultiInstrumentWatcher` in
`wickd-core`). If you write or maintain a strategy script, or you change
the Rhai engine in `crates/wickd-core/src/backtest/scripted_strategy.rs`,
this document — and its "ABI version" above — is the thing to keep in sync.

A CI test (`crates/wickd-core/tests/golden_script_corpus.rs`) runs every
worked example under `crates/wickd-core/tests/golden_scripts/` through
`validate_script` on every build, so this document cannot silently drift from
what the engine actually accepts. If you change a documented example here,
update the matching `.rhai` file in that directory too (see "Golden corpus"
below).

## Where scripts live

**`~/.wickd/strategies/` is the unified strategy store** (AGT-651): one
`.rhai` file per strategy, the file stem is the strategy's canonical name,
and the `@indicators`/`@parameters` metadata comments are the single source
of truth — both the wickd CLI and the desktop app list/read/run the same
directory (the app is a read-only viewer/runner; authoring happens through
the CLI or your editor). See `wickd_core::strategy_store::StrategyStore` and
`docs/strategy-store.md` for the store API and the full CLI lifecycle
(`wickd strategy add / show / update / remove / convert`).

`wickd strategy run` / `wickd backtest` accept a `strategy` argument that is
resolved (see `crates/wickd/src/commands/scripted.rs`) in this order:

1. An **explicit path** — a literal file that exists on disk, or anything
   ending in `.rhai` (even if missing, so a typo produces a clear "file not
   found" error rather than silently falling through).
2. A **built-in name** (`ma-crossover`, `rsi`) — checked before bare-name
   script lookup, so a script can never shadow a built-in.
3. A **bare name** resolved to `~/.wickd/strategies/<name>.rhai`.

Every script is passed through `validate_script` before it is ever loaded into
a running strategy — that call compiles the script, checks for a valid
`on_candle()` function, and parses the `@indicators`/`@parameters` metadata
comments. A malformed script fails fast with a message naming the offending
file; it never panics and never silently loads a partial strategy.

## The `on_candle()` contract

A script **must** define:

```rhai
fn on_candle() {
    // ... return a signal ...
}
```

`on_candle()` is called once per candle close, after indicators have been
updated for that candle. It must return either:

- **A plain string signal** — one of `"buy"` / `"long"`, `"sell"` / `"short"`,
  `"close"` / `"close_position"`, or anything else (treated as `"hold"`). This
  is the simplest valid return and is enough for a script with no risk levels.
- **An object map** with these fields (all optional except `signal`):

  | Field | Type | Meaning |
  |---|---|---|
  | `signal` | string | Same vocabulary as the plain-string form. Missing → `Hold`. |
  | `stop_loss` | Decimal | Stop-loss price level, if the strategy knows one. |
  | `take_profit` | Decimal | Take-profit price level. |
  | `rule_name` | string | Name of the entry rule that fired (for `buy`/`sell`). |
  | `exit_reason` | string | Human-readable reason for a `close` signal. |
  | `pending_order` | map | Optional (engine support since AGT-607). Turns a `buy`/`sell` signal into a **pending order** instead of a market entry: `#{ order_type: "buy_stop"\|"sell_stop"\|"buy_limit"\|"sell_limit", price: Decimal, expiry_bars: int? }`. The order fills when price reaches `price` (spread-adjusted); `expiry_bars` cancels it after N unfilled bars (omit = no expiry). A new signal replaces any outstanding pending order; a malformed map (missing `order_type`/`price`, unknown type string) degrades the signal to a plain market entry, consistent with the "typos degrade to inert" convention. |

  ```rhai
  #{ signal: "buy", stop_loss: sl, take_profit: tp, rule_name: "EMA cross" }
  // Pending (limit) entry instead of market:
  #{ signal: "buy", stop_loss: sl,
     pending_order: #{ order_type: "buy_limit", price: close - atr * 0.25, expiry_bars: 3 } }
  ```

A script **may** also define an optional lifecycle hook:

```rhai
fn on_position_closed() {
    // Called when the engine closes a position the script opened (SL/TP
    // fill, forced close at end of backtest, etc.) — clear any internal
    // position-tracking state here.
}
```

**Top-level script code** (anything outside a function) runs exactly once,
when the strategy is constructed or `reset()`, to initialize global state
(e.g. `let position_open = false;`). It is bound by the same resource-safety
limits as `on_candle()` (see below) — a slow or runaway top-level block is
aborted the same way a runaway `on_candle()` call is.

## The `@indicators` / `@parameters` metadata format

Declared as JSON following a `// @indicators:` or `// @parameters:` comment.
The JSON may span multiple consecutive `//` comment lines; parsing stops at
the first non-comment line or once brackets/braces balance.

```rhai
// @indicators: [
//   { "id": "ema_fast", "type": "ema", "params": { "period": { "$param": "fast_period" } } },
//   { "id": "rsi", "type": "rsi", "params": { "period": 14 } }
// ]
// @parameters: [
//   { "id": "fast_period", "name": "Fast EMA Period", "type": "integer", "default": 9, "min": 3, "max": 50, "step": 1 }
// ]
```

**`@indicators`** — array of indicator declarations:

| Field | Required | Meaning |
|---|---|---|
| `id` | yes | Unique id, referenced from `on_candle()` via `indicator(id, output)` etc. |
| `type` | yes | Snake_case indicator type (`ema`, `sma`, `rsi`, `atr`, `macd`, ...). |
| `params` | no | Object of indicator params. Each value is either a fixed number, or `{"$param": "<parameter id>"}` to bind it to a `@parameters` entry so users can tune it without editing the script. |

**`@parameters`** — array of tunable parameter declarations:

| Field | Required | Meaning |
|---|---|---|
| `id` | yes | Unique id, read from `on_candle()` via `param(id)`. |
| `name` | no (defaults to `id`) | Display name. |
| `description` | no | Help text. |
| `type` | no (default `"number"`) | `"integer"` \| `"number"`/`"float"` \| `"select"` \| `"boolean"`/`"bool"`. |
| `default` | no (default `0`) | Default value if not overridden. |
| `min` / `max` / `step` | no | Numeric bounds/step for UI sliders and CLI validation. |
| `group` | no | UI grouping label. |
| `options` | no | For `"select"` params: array of `{ "value": number, "label": string }` **objects** — a plain string array is silently filtered to `[]` by the parser (no error, no warning), so always use the object form. Select values are numeric by construction (`param()` returns numbers). |

Two parser gotchas worth knowing (probe-verified, AGT-637 D7/D8):

- **Boolean defaults must be numeric.** `"default": true` parses as `0.0`
  silently — write `"default": 1` / `"default": 0` for boolean parameters.
- **Select options must be `{value, label}` objects** (see the `options` row
  above); string arrays are dropped without a diagnostic.

A script with no metadata comments at all is valid — `@indicators` and
`@parameters` both default to empty. See `01_minimal.rhai` in the golden
corpus.

## SDK function reference

29 functions make up the authoring surface: **27 native functions** the host
registers on the Rhai engine, plus the **2 script-defined lifecycle
functions** above (`on_candle()` required, `on_position_closed()` optional).

All price/indicator/parameter values are `Decimal` (via the `rust_decimal`
crate — `rhai`'s `decimal` feature), not floating point, so results are exact
and comparable directly against float literals in script source (e.g.
`rsi_val > 70.0`).

| Function | Signature | Returns |
|---|---|---|
| `price(field)` | `field: "open"\|"high"\|"low"\|"close"` | Current candle's price at that field. |
| `price_at(field, offset)` | `offset: int`, 0 = current candle counting back | Historical price, oldest-first internally but indexed from the current candle backward. |
| `indicator(id, output)` | indicator id, output name (usually `"value"`) | Current value of that indicator output. |
| `indicator_at(id, output, offset)` | `offset: int`, 0 = current | Historical indicator value (up to 10 candles of lookback). |
| `param(id)` | parameter id | Resolved parameter value (default, or CLI/caller override). |
| `bar_count()` | — | 1-based count of candles processed so far. |
| `volume()` | — | Current candle's volume. |
| `pip_value()` | — | Pip size for the instrument (0.01 for JPY pairs, 0.1 for XAU, 0.001 for XAG, else 0.0001). |
| `candle_time()` | — | The current candle's open time as Unix seconds (UTC, `int`). The script's clock — sound across weekend/missing-candle gaps where bar-index arithmetic is not. 0 before the first candle. |
| `candle_hour()` | — | The current candle's open hour, 0–23 UTC (`int`). Convenience for session gates. |
| `hours_since_event()` | — | Hours (`Decimal`, fractional) since the most recent economic-calendar event at or before this candle's open; **-1** when no calendar is loaded or no prior event exists. See "Event calendar" below. |
| `hours_until_event()` | — | Hours (`Decimal`) until the next calendar event after this candle's open; **-1** when unknown. |
| `surprise_z()` | optional `min_impact: "high"\|"medium"\|"low"` (default `"high"`) | Signed actual-vs-forecast z-score (`Decimal`) of the most recent *scored* release at or before this candle's open, on either of the instrument's currency legs; **-9999** when none. See "Surprise feed" below. |
| `surprise_z_for(currency)` | `currency: "USD"\|...`, optional `min_impact` | Same, but for one explicit currency (need not be an instrument leg). |
| `surprise_hours_ago()` | optional `min_impact` | Hours (`Decimal`) since the release the matching `surprise_z()` call refers to (same filters ⇒ same release); **-1** when none. |
| `surprise_hours_ago_for(currency)` | `currency`, optional `min_impact` | Same, for one explicit currency. |
| `is_bullish()` | — | `true` if `close > open` for the current candle. |
| `is_bearish()` | — | `true` if `close < open` for the current candle. |
| `candle_range()` | — | `high - low` for the current candle. |
| `body_size()` | — | `\|close - open\|` for the current candle. |
| `crossed_above(id1, out1, id2, out2)` | two indicator refs | `true` if `id1.out1` crossed above `id2.out2` on this candle. |
| `crossed_below(id1, out1, id2, out2)` | two indicator refs | `true` if `id1.out1` crossed below `id2.out2` on this candle. |
| `crossed_above_value(id, output, value)` | indicator ref + fixed `Decimal` | `true` if the indicator crossed above a fixed value. |
| `crossed_below_value(id, output, value)` | indicator ref + fixed `Decimal` | `true` if the indicator crossed below a fixed value. |
| `in_position()` | — | **ABI v5.** `true` while the backtest engine holds an open position for this strategy. Always `false` on hosts that don't simulate positions (live watcher, `wickd strategy run`) — see "Position state" below. |
| `entry_price()` | — | **ABI v5.** The actual fill price (`Decimal`) of the open position; **0** when flat — gate with `in_position()` first. |
| `bars_since_entry()` | — | **ABI v5.** Completed candles (`int`) since the position opened (0 on the fill candle); **-1** when flat. |

If an indicator/parameter id doesn't resolve (typo, or referenced before the
first candle populates it), these functions return a safe zero value
(`Decimal::ZERO` / `false` / `0`) rather than erroring — a script that
misspells an id degrades to inert output instead of crashing.

See `04_full_sdk_surface.rhai` in the golden corpus for a script that calls
every one of these at least once.

## Event calendar (ABI v3)

`hours_since_event()`/`hours_until_event()` are fed from one source of truth,
injected at strategy construction (`wickd backtest`, `wickd strategy run`, and
the live watcher whenever it runs a scripted strategy):

1. **`~/.wickd/events.json`** when present — your calendar, same schema as
   the bundled file:
   ```json
   { "events": [ { "time_utc": "2022-01-26T19:00:00Z",
                   "currency": "USD", "type": "rate_decision", "name": "FOMC" } ] }
   ```
   A corrupt user file is a hard error; a missing one falls back to:
2. **The bundled calendar** (`crates/wickd-core/assets/events.json`, compiled in):
   FOMC, ECB, and BoE rate decisions plus US NFP and CPI releases,
   **2022-01 → 2026-06**, from published schedules (cited in the file's
   `sources` field).

Events are filtered to the instrument's two currency legs (EUR_USD keeps EUR
and USD events, drops GBP). Candles outside the bundled span simply see a
growing `hours_since_event()` (or -1 before the first event) — extend
coverage via the user file.

**Honesty requirement for event-window backtests:** the engine's spread cost
is FLAT (`BacktestConfig.spread_pips`), but real spreads blow out around
news. A backtest that trades inside event windows therefore *overstates* the
edge — treat event-window results as optimistic until validated with a wider
spread assumption (a configurable event-window spread is a known follow-up,
not yet implemented).

## Surprise feed (ABI v4)

`surprise_z()` / `surprise_z_for()` / `surprise_hours_ago()` /
`surprise_hours_ago_for()` expose economic-release *surprises* — the
actual-vs-forecast z-score of the most recent release at or before the
current candle's open. This is the production path for surprise-conditioned
strategies (H-015-live, fx-tracker #290 follow-up): the engine finally sees
its own entry condition at runtime instead of scripts embedding precomputed
datelines.

### The calendar: `~/.wickd/calendar/*.csv` — updatable without rebuilding

The feed reads every `*.csv` in **`~/.wickd/calendar/`** — manual monthly
ForexFactory exports with the exact header
`date,time,currency,event,impact,actual,forecast,previous` (`date` is
`YYYY-MM-DD`, `time` is `HH:MM` UTC). Seed it from the research corpus:

```
mkdir -p ~/.wickd/calendar
cp <fx-tracker>/data/calendar/*.csv ~/.wickd/calendar/
```

Dropping a new month — or re-dropping an existing month with backfilled
`actual` values — updates the feed **without rebuilding wickd**:

- a fresh `wickd backtest` / `wickd strategy run` reads the directory at
  strategy construction;
- a long-lived process (the watcher) re-checks file fingerprints from the
  candle path at most once per 60s and reloads when any CSV was added,
  removed, or modified. A half-written or malformed drop is logged and the
  previous data kept — it never kills a running watcher.

A missing directory is an empty feed (accessors sit at their sentinels), so
the engine works before the first drop. A CSV whose *header* doesn't match
is a hard load error naming the file (wrong format is user error); rows with
blank or unparseable cells are normal data gaps and simply skipped. Numeric
cells accept ForexFactory's `%`/`K`/`M`/`B`/`T` suffixes and `<`/`>`
qualifiers.

### Z-score methodology — pre-committed stats, no lookahead

A release's surprise is `actual − forecast`, z-scored per event series
(keyed on event name + currency) against that series' mean and population
standard deviation computed **only from releases before the frozen
discovery cutoff, 2025-01-01T00:00:00Z** — the wickd-lab discovery window,
mirroring STUDY-003 / the H-015 registration. A series needs ≥8 discovery
releases and non-zero variance to be scoreable; unscoreable series are
invisible to the accessors. Because the stats are frozen at the cutoff, a
z-score read at candle time never encodes post-cutoff information beyond
the release's own published numbers, and releases only become visible at or
after their release timestamp (candle *open* time is the clock, consistent
with `hours_since_event()`).

### Filters and sentinels

- **Default filters:** `min_impact = "high"`, currency ∈ the instrument's
  two legs (EUR_USD sees EUR and USD releases) — the same leg convention as
  the v3 event calendar. `min_impact` widens to `"medium"`/`"low"`;
  `*_for(currency)` pins one explicit currency (any currency, not just a
  leg). An unknown `min_impact` label matches nothing (sentinels), per the
  ABI's "typos degrade to inert" convention.
- **Sentinels:** `surprise_z()` returns **-9999** and `surprise_hours_ago()`
  returns **-1** when no scored release matches: no calendar dropped, no
  release at or before the candle, filters match nothing — or the latest
  release is not scoreable.
- **Forecast but no actual yet:** a release whose `actual` is not yet
  published is **not a surprise yet** — it is skipped entirely, and the
  accessors report the most recent *scored* release instead (which may be
  days older; gate on `surprise_hours_ago()`). Once a re-dropped CSV
  backfills the `actual`, the release becomes visible with its z-score on
  the next reload. The accessors never return a partial answer for a
  pending release.
- **Same release guarantee:** `surprise_z(...)` and
  `surprise_hours_ago(...)` with the same filters always refer to the same
  release, so a script can gate a z-score on its recency without a race.

The flat-spread honesty caveat above applies doubly here: surprise windows
are exactly where spreads blow out.

See `07_surprise_fade.rhai` in the golden corpus for the worked H-015-live
example.

## Position state (ABI v5)

`in_position()`, `entry_price()`, and `bars_since_entry()` expose the
**backtest engine's** open-position truth to scripts, closing the ABI's one
systematic expressiveness gap vs the retired rules DSL (risk/reward and
bar-count exits used to require self-tracked approximations that were off by
the signal→fill gap).

How the engine feeds it: once per candle — after pending-order fills,
market-signal execution, and SL/TP checks are settled, immediately before
`on_candle()` — the engine pushes its current position snapshot into the
strategy (`Strategy::sync_position_state`). So inside `on_candle()`:

- `in_position()` — `true` iff a simulated position is open *right now*.
- `entry_price()` — the actual fill (next-candle open ± spread, or the
  pending-order price), not the signal-candle close. `0` when flat.
- `bars_since_entry()` — `0` on the fill candle, incrementing each completed
  candle; `-1` when flat.

Inside `on_position_closed()` the position is already gone: `in_position()`
is `false` and `bars_since_entry()` is `-1`.

**Hosts without an engine.** The live watcher and `wickd strategy run`
evaluate scripts against streaming candles without simulating fills, so the
accessors sit at their flat sentinels there (`false` / `0` / `-1`) — same
convention as the v3/v4 sentinels. A script that gates its exit logic behind
`in_position()` is therefore automatically inert in watch mode (the watcher
surfaces entry signals; position management belongs to the execution layer).

See `08_rr_exit_position_state.rhai` in the golden corpus for the worked
RR-exit example, and `crates/wickd-core/tests/scripted_position_state.rs`
for the pinned engine semantics.

## Resource-safety limits (AGT-606)

Every Rhai engine the host constructs — for `validate_script` *and* for
running a script live — gets its limits from the exact same function
(`configure_engine_limits` in `scripted_strategy.rs`), so a script that
validates is guaranteed to run under the same limits, and there is no way for
the two to drift apart:

| Limit | Value | What it bounds |
|---|---|---|
| Max operations | 1,000,000 | Total Rhai "operations" (statements/expressions) per `on_candle()` call. |
| Max call levels | 32 | Function call / recursion depth. |
| Max expression depth | 64 (both top-level and in-function) | Nesting depth of a single expression. |
| Max string size | 10,000 bytes | Any single string value. |
| Max array size | 10,000 elements | Any single array value. |
| Max object-map size | 1,000 keys | Any single object-map value. |

**Wall-clock budget — the daemon's real anti-hang guard.** Operation count
bounds *work*, but a script can spend its operation budget on something
individually slow and still take far longer, wall-clock, than a live watcher
can tolerate between candles. Every script entry-point call (`on_candle()`,
`on_position_closed()`, and the one-time top-level init) is additionally
bounded to **~50ms**, checked periodically via Rhai's `on_progress` hook. A
script that blows the budget is terminated — the call returns an error, is
logged, and the strategy reports `Hold` for that candle — instead of hanging
the process.

**Consecutive-error abort threshold.** If `on_candle()` raises 50 consecutive
errors (compile-time-valid scripts can still fail at runtime — e.g. a
division by zero, or repeatedly hitting the wall-clock/array/map limits above),
the strategy stops calling into the script entirely and reports `Hold` on
every subsequent candle, permanently, until the strategy is reset (a fresh
`wickd strategy run`/`backtest` invocation, or the watcher restarting it).

This "Hold forever" fallback is intentional — a broken script should not keep
retrying indefinitely — but a bare `Hold` looks identical to a healthy
strategy legitimately finding no signal. To make the two distinguishable, the
first candle that crosses the threshold emits an explicit health event:

- Programmatic callers of `ScriptedStrategy` can call `take_abort_event()`
  (edge-triggered — `true` exactly once per abort, `false` after, until
  `reset()` re-arms it) and `abort_reason()` for a human-readable message.
- The live watcher (`StrategyWatcher` / `MultiInstrumentWatcher` in
  `wickd-core::strategy`) surfaces this as a `strategy_error` /
  `strategy-error` event with `error_type: "script_aborted"` — distinct from
  `"tick_error"`, `"transient_error"`, and `"warmup_failed"` — through the same
  `EventSink` the rest of the watcher's error reporting uses.

## Agent authoring loop (AGT-609)

Matt never hand-edits `.rhai` scripts — he describes a strategy in natural
language and a driver agent authors, validates, backtests, and iterates on it.
This section is the contract that agent follows. Every step is a single command
that prints one JSON object to stdout (compact by default, `--pretty` for
indented) and exits 0 on success, so the agent branches on parsed fields, never
on prose. See `crates/wickd/src/output.rs` for the exit-code contract.

### The loop: generate → validate → backtest → iterate

1. **Generate.** Author a `.rhai` script from the NL request, following the
   `on_candle()` contract, the `@indicators`/`@parameters` format, and the SDK
   function reference above. Write it to a path (a scratch file, or
   `~/.wickd/strategies/<name>.rhai` to make it resolvable by bare name).

2. **Validate** (no network) — cheap, do it first and on every edit:

   ```
   wickd strategy validate ./my-strategy.rhai
   ```

   Resolution matches `run`/`backtest` (explicit `.rhai` path → built-in name →
   `~/.wickd/strategies/<name>.rhai`). Output for a script that compiles:

   ```json
   {
     "strategy": "./my-strategy.rhai",
     "kind": "script",
     "path": "./my-strategy.rhai",
     "valid": true,
     "score": 100,
     "errors": [],
     "warnings": [],
     "metadata": {
       "indicators": [ { "id": "ema_fast", "type": "ema", "params": { "period": 9 } } ],
       "parameters": [ { "id": "fast", "name": "Fast EMA", "type": "integer", "default": 9, "min": 3, "max": 50, "step": 1 } ]
     }
   }
   ```

   Output for a script that does **not** compile — still exit 0, so the agent
   reads the structured error and fixes the script rather than parsing a
   stack-trace:

   ```json
   {
     "strategy": "./my-strategy.rhai",
     "kind": "script",
     "path": "./my-strategy.rhai",
     "valid": false,
     "score": 0,
     "errors": [ { "code": "compile_error", "message": "Script compilation error: ..." } ],
     "warnings": []
   }
   ```

   Fields:

   | Field | Meaning |
   |---|---|
   | `valid` | `true` iff the script compiled **and** defines `on_candle()`. Gate the rest of the loop on this. |
   | `score` | `0` when invalid; otherwise `100` minus `10` per warning. A single deterministic number to rank variants by authoring quality. |
   | `errors` | Array of `{ code, message }`. Empty when `valid`. `code` is stable (see below); `message` is the human string, preserved verbatim. |
   | `warnings` | Array of `{ code, message }` — non-fatal authoring smells (e.g. `no_parameters`, which means the script can't be tuned or walk-forward-optimized). |
   | `metadata` | Parsed `@indicators` / `@parameters` (present only when `valid`). Tells the agent exactly what's tunable. |
   | `kind` | `"script"` for a `.rhai` file, `"builtin"` for `ma-crossover`/`rsi` (always `valid`, `score` 100). |

   Stable `errors[].code` values — one per `ScriptValidationError` variant in
   `wickd-core`, so they can't drift from message wording: `compile_error`
   (script didn't compile), `missing_on_candle` (no `on_candle()` function),
   `metadata_error` (`@indicators`/`@parameters` failed to parse). Branch on
   `code`, not `message`.

3. **Backtest** (network — fetches OANDA candles) once `valid` is `true`:

   ```
   wickd backtest ./my-strategy.rhai EUR_USD --granularity H1 --count 1000
   ```

   Scripted and built-in strategies produce the **same** result shape:

   ```json
   {
     "strategy": "./my-strategy.rhai",
     "instrument": "EUR_USD",
     "granularity": "H1",
     "candles": 1000,
     "metrics": {
       "totalPnl": "123.45", "totalReturnPct": "1.23", "annualizedReturnPct": "...",
       "winningTrades": 12, "losingTrades": 8, "winRate": "60", "avgWin": "...",
       "avgLoss": "...", "profitFactor": "1.8", "maxDrawdownPct": "4.2",
       "sharpeRatio": "0.9", "totalTrades": 20
     },
     "finalBalance": "10123.45",
     "equityCurve": ["10000", "..."],
     "trades": [ /* per-trade records */ ]
   }
   ```

   All money/ratio fields in `metrics` are JSON **strings** (exact `Decimal`,
   never lossy floats); trade counts (`winningTrades`, `losingTrades`,
   `totalTrades`) are numbers. `metrics` is the object the agent compares
   variants on — `totalPnl`, `winRate`, `profitFactor`, `maxDrawdownPct`, and
   `sharpeRatio` are the usual objective fields.

   To pressure-test that the *fitting process* generalizes (not just one fixed
   parameter set), add `--walk-forward`; that reports per-window in-sample vs
   out-of-sample `metrics` under a `walkForward` key — see AGT-608 and
   `crates/wickd/src/commands/walk_forward.rs`.

4. **Iterate.** Read `metrics`, adjust the script or its `@parameters`, and go
   back to step 2. Because `validate` is free and offline, re-validate after
   every edit and only spend a network backtest on a script that already
   passes.

### Propose-and-compare (2–3 variants side by side)

Matt wants to compare *options*, not watch a single number get nudged. So the
driver agent's default authoring move is to produce **2–3 distinct variants**
of the strategy — not one tweak — and backtest them side by side over the same
instrument, granularity, and range, so the comparison is apples-to-apples:

1. Author variants `A`, `B`, `C` that differ on a *meaningful* axis (e.g. a
   trend-follower vs a mean-reverter; a tight vs a loose stop; RSI(14) vs
   RSI(7)), not cosmetically.
2. `wickd strategy validate` each — drop any that aren't `valid` before
   spending a backtest, and note the `score`/`warnings`.
3. `wickd backtest <variant> EUR_USD --granularity H1 --count 1000` each, with
   **identical** instrument / `--granularity` / `--count` (or `--from`/`--to`)
   so the runs are comparable. Optionally `--walk-forward` each to see which
   variant's edge survives out-of-sample.
4. Assemble a small table from the parsed `metrics` — one row per variant, with
   `totalPnl`, `winRate`, `profitFactor`, `maxDrawdownPct`, `sharpeRatio`,
   `totalTrades` — and present it to Matt so he picks. Surface the trade-offs
   (e.g. "B has the highest P&L but the deepest drawdown; C is steadier"),
   don't silently declare a winner.

Because both surfaces are JSON, the whole loop — generate, validate, backtest
N variants, tabulate — is scriptable end to end with no prose parsing.

## Golden corpus

`crates/wickd-core/tests/golden_scripts/` holds the worked examples this
document references, and `crates/wickd-core/tests/golden_script_corpus.rs`
runs every one of them through `validate_script` on every `cargo test`:

| File | Demonstrates |
|---|---|
| `01_minimal.rhai` | No metadata; plain-string signal return. |
| `02_rsi_with_parameters.rhai` | `@indicators` + `@parameters`, including a `{"$param": ...}` reference. |
| `03_ema_cross_with_risk.rhai` | Extended `#{...}` signal with `stop_loss`/`take_profit`/`rule_name`/`exit_reason`, and `on_position_closed()`. |
| `04_full_sdk_surface.rhai` | Every native SDK function above, called at least once. |
| `05_session_gate.rhai` | The v2 candle clock: `candle_hour()` session gating + `candle_time()` elapsed-time gating (with parameterized session bounds). |
| `06_event_blackout.rhai` | The v3 event calendar: `hours_since_event()`/`hours_until_event()` gating a reversion strategy out of post-event windows. |
| `07_surprise_fade.rhai` | The v4 surprise feed: `surprise_z()`/`surprise_hours_ago()` gating an RSI fade to the window right after a big (`\|z\| > 1.5`) high-impact surprise, with sentinel handling. |
| `08_rr_exit_position_state.rhai` | The v5 position state: `in_position()`/`entry_price()`/`bars_since_entry()` driving exact RR + stale exits, plus a `pending_order` limit entry. |

## Version history

- **v1** (2026-06, AGT-606) — First versioned publication of this document.
  Codifies the pre-existing `on_candle()`/`on_position_closed()` contract and
  `@indicators`/`@parameters` format (introduced in AGT-605), and adds the
  `max_array_size`/`max_map_size` limits, the wall-clock budget, and the
  `script_aborted` health event.
  - **AGT-609** (2026-06) — Added the "Agent authoring loop" section: the
    `wickd strategy validate` JSON surface (`valid`/`score`/`errors[]`), the
    `wickd backtest` metrics shape, and the generate → validate → backtest →
    iterate + 2–3-variant propose-and-compare workflow a driver agent follows.
    No change to the `on_candle()` ABI or engine limits, so the version stays 1.
- **v2** (2026-07, #289) — Added the candle clock: `candle_time()` (Unix
  seconds of the candle's open, UTC) and `candle_hour()` (0–23 UTC). Native
  SDK surface grows 16 → 18. Both return safe zeros before the first candle,
  consistent with the existing unresolved-id semantics. No changes to
  lifecycle functions, metadata format, or engine limits.
- **v3** (2026-07, #290) — Added the event calendar: `hours_since_event()`
  and `hours_until_event()` (`Decimal` hours, -1 when unknown), fed from
  `~/.wickd/events.json` or the bundled 2022–2026 FOMC/ECB/BoE/NFP/CPI
  schedule, filtered to the instrument's currency legs. Native SDK surface
  grows 18 → 20. Includes the flat-spread honesty caveat for event-window
  backtests.
- **v4** (2026-07, AGT-632) — Added the live surprise feed: `surprise_z()`,
  `surprise_z_for()`, `surprise_hours_ago()`, `surprise_hours_ago_for()`
  (signed z of `actual − forecast` against frozen pre-2025 discovery stats;
  sentinels -9999 / -1), fed from updatable monthly ForexFactory CSVs in
  `~/.wickd/calendar/` — droppable without rebuilding wickd, picked up by
  long-lived processes via a throttled per-candle fingerprint check. A
  release with a forecast but no published actual yet is skipped until
  backfilled. Native SDK surface grows 20 → 24. No changes to lifecycle
  functions, metadata format, or engine limits.
- **v5** (2026-07, AGT-651) — Added engine-fed position state:
  `in_position()`, `entry_price()`, `bars_since_entry()` (flat sentinels
  `false` / 0 / -1; real values only inside a `BacktestEngine` run). Native
  SDK surface grows 24 → 27. Also documented two things the engine already
  supported but this document didn't: the `pending_order` signal-map field
  (engine support since AGT-607) and the D7/D8 metadata parser gotchas
  (object-form select options, numeric boolean defaults). Declared
  `~/.wickd/strategies/` the unified strategy store shared with the desktop
  app. No changes to lifecycle functions or engine limits.
