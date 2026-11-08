# wickd

Headless, agent-first CLI for OANDA trading. The CLI is the *hands and eyes* —
it pipes clean OANDA data in (candles, instruments, live prices) and sends
orders out. The *brain* (backtesting, pattern-matching, strategy) lives in the
agent wrapping the binary, which reasons over the JSON the CLI emits.

Output is JSON by default (NDJSON for `stream`); `--pretty` switches to indented
JSON.

New here? [`WALKTHROUGH.md`](./WALKTHROUGH.md) is a hands-on, guided tour of every
feature — meant to be followed together by you and an agent.

## Verbs

| Verb | Purpose |
|------|---------|
| `login` / `logout` | Store / remove OANDA credentials (API key in the OS keychain) |
| `candles` | Historical OHLC (+ indicators) for an instrument |
| `instruments` | List tradeable instruments |
| `trade` | Account, positions, orders, order execution |
| `stream` | Live prices → JSON-lines |
| `strategy` | List / run built-in strategies over candles (or a Rhai `.rhai` script — see [`STRATEGY_ABI.md`](./STRATEGY_ABI.md)) |
| `view` | Open an on-demand GUI view (FX ticket / live signal watcher); headless otherwise |
| `watch` | Persistent, monitoring-only signal daemon (NDJSON events; never trades) |
| `audit` | Read the append-only audit log of execution decisions |
| `trade report` / `trade baseline` | Account P&L since a recorded per-account baseline |
| `tui` | Stock ticker TUI: watchlist, symbol search, price-history drill-down (US equities) |
| `alpaca login` / `status` / `logout` | Store / inspect / remove the Alpaca market-data keys `tui` uses |

## `wickd tui` — stock ticker

An interactive terminal ticker for US stocks and ETFs, backed by Alpaca's
market-data API. Like `dashboard`, it is interactive rather than JSON. It is
read-only: it never places orders.

```
wickd alpaca login     # once: key id + secret from a free Alpaca paper account
wickd tui              # IEX real-time quotes (free plan)
wickd tui --feed sip   # consolidated real-time quotes (paid Algo Trader Plus)
```

- **Keys** are Helix-style. Bare keys move (`j`/`k`, arrows), open (`enter`),
  back out (`esc`), search (`/`), refresh (`r`) and quit (`q`). A prefix letter
  opens a category, and a panel lists the next keys:
  - `g` go to: `g g` top, `g e` end, `g <n> g` row n, `g w` watchlist, `g i`
    in play today.
  - `w` watchlist: `w a` add, `w d` remove, `w u` undo, `w j`/`w k` move.
  - `t` time range: `t d`/`t w`/`t m`/`t 6`/`t y`/`t 5`/`t a` for
    1D/5D/1M/6M/1Y/5Y/MAX.
  - `v` view: `v c` candles, `v l` line, `v t` price table, `v e` extended
    hours.

  On a chart, `h`/`l` (or arrows) move a cursor candle by candle, `H`/`L` pan
  half a screen, `+`/`-` zoom, `0` returns to fit, `[`/`]` change range, and
  `g g`/`g e` jump to the oldest or latest candle.

  `esc` cancels a pending prefix, and `?` lists every key. The help screen,
  the footer and the panel are all generated from the one keymap table
  (`src/tui/keymap.rs`).
- **Watchlist** (`~/.wickd/tui.json`). Shows last, change vs the prior session
  close, day range, volume and a sparkline of today.
- **In play** (`g i`). Lists what is worth watching today. It merges top
  movers, the most active names and your watchlist, then shows for each name:
  - change against yesterday's close (the gap, in pre-market);
  - the move since the open;
  - volume so far and relative volume: today against the 20-day average, from
    the same feed, so it means something even on IEX;
  - how recent its latest headline is.

  `v c`, `v r` and `v a` sort by biggest move, relative volume or volume, and
  `v f` toggles the price ≥ $5 filter. `w a` adds the selected name to the
  watchlist, and `enter` opens its chart. A side pane shows Alpaca/Benzinga
  headlines for the selected symbol. Sources your plan doesn't include show a
  note instead of breaking the screen. Top movers only reset at the open, so
  before then the gap column is the useful one.

  **Reading articles.** `tab` moves focus into the headlines, `j`/`k` picks a
  headline (`↗` means it has a link), and `enter` opens the article in a
  chromeless ui-leaf window. The window has its own keys: **ctrl+shift+→ /
  ←** load the next or previous linked headline, and a corner badge shows
  where you are (`NVDA · 3 of 12`). The TUI's `j`/`k` also drive it. `x`
  closes the window, and `tab` or `esc` returns to the list. If you close
  the window yourself, the next article just opens a new one. wickd drives the window over Chrome's DevTools
  protocol, on a random loopback port with a throwaway Chrome profile, because
  Benzinga pages refuse to be embedded in another page. That port has no
  authentication while the window is open, so only use it on your own machine.
  It needs `ui-leaf` on your PATH.
- **Search**. Matches ticker or company name over Alpaca's active-equity list,
  cached daily in `~/.wickd/cache/alpaca-assets.json`. `tab` adds or removes a
  symbol, `enter` opens it.
- **Symbol view**. Covers 1D to MAX (`1`–`7` are shortcuts for the ranges).
  The default is a candlestick chart using the desktop app's hollow-candle
  scheme. Colour shows direction against the previous candle's close (teal
  up, red down). The body is hollow when the close is at or above the open,
  and solid when it's below.
  - The chart opens in **fit**: the whole range spread across the width. When
    there are more bars than columns, consecutive bars merge into one candle
    with their real open, high, low and close, and the title shows how many
    (`× 4`).
  - Zooming in widens candles to 2, 4 and 6 columns. Wide hollow candles draw
    as outlines. Zooming out merges 2, 4 or 8 bars per candle.
  - A readout line shows the cursor candle's time, OHLC, change and volume.
  - The price axis on the right has gridlines and a tag for the last price.
  - Volume sits underneath, scaled to the 95th percentile so one
    closing-auction bar doesn't flatten the rest.
  - Intraday ranges show regular hours (09:30–16:00 ET) unless you toggle
    `v e`. On 1D, a dashed line marks the prior close.

  `v l` switches to a close-price line with the same zoom and cursor, and
  `tab` flips between the last chart style and a newest-first OHLCV table.
- **Data**. Quotes poll every 15s (`--refresh`). On the free plan, live quotes
  are IEX-only (about 3% of volume). `--feed delayed-sip` trades that for
  all-exchange quotes 15 minutes behind. History is always consolidated and
  split-adjusted, and runs 15 minutes behind on the free plan. On the free
  plan the chart title says `15m delayed`, and a dotted marker joins the
  newest candle to the live price tag, so the gap reads as "moved since".
  Times are shown in the local zone.
- **Keys**. Agents can export `APCA_API_KEY_ID` / `APCA_API_SECRET_KEY`
  instead of using the keychain item (service `wickd`, account `alpaca`).

## Named accounts (`--account`)

One OANDA practice login fans out into several practice *sub-accounts* — one
per strategy. `wickd login --env practice --account h004 --account-id <id>`
registers a named account; with no `--api-key` it reuses the environment's
shared token from the keychain (the primary shape — many sub-account ids under
one OANDA login token), while `--api-key` stores a dedicated keychain item for
that account. `--account <name>` on `trade`, `watch`, `approve`, and `stream`
then selects whose credentials to use. Everything defaults to the account named
`default`: an existing single-account config keeps working unchanged, no
migration step. `wickd login --status` lists configured environments and
account names (no secrets).

To run `wickd watch` as a supervised, auto-restarting service on the always-on
host, see [`deploy/launchd/`](./deploy/launchd/README.md) (macOS launchd
LaunchAgent + install/uninstall scripts).

## Driving wickd from a local agent

wickd is the *hands and eyes*; the *brain* is a **local Claude agent that shells
out to the `wickd` binary**, reads its JSON, reasons, and issues the next
command. The agent wraps the binary as a subprocess — **no model is embedded in
this Rust CLI**, by design.

The loop is **data → reason → orders**:

1. **Data** — `wickd strategy run <strategy> <instrument> …` (and/or `wickd
   backtest …`, `wickd trade account`) → parse the JSON object on stdout.
2. **Reason** — feed that JSON to a Claude agent step (e.g. the `claude` CLI in
   headless `-p` mode, the Anthropic SDK, or a local LLM) and get back a decision.
3. **Orders** — on a decision to trade, `wickd trade place --instrument … --units …`
   and act on the result. Loop.

**Paper by default.** Order-submitting verbs are paper/dry-run unless `--live` is
explicitly armed (see `wickd trade`'s execution-safety note). An agent loop should
default to paper — emit the would-be order JSON (`"mode":"paper","submitted":false`)
and only arm `--live` behind a deliberate, confirmed step.

**Autonomous practice execution (AGT-626).** A live submit normally needs a human
TTY keystroke, which blocks headless autonomy. For **practice accounts only**,
`--live --auto` (or the programmatic `execute_place_auto` / `execute_close_auto`
entry points) arms a real submit without a TTY — the trust-ladder Stage 2 path.
It is refused (fail-closed) on `--env live`: autonomy can never fire a real-money
order. The full guarded contract (pre-submit audit row, risk caps, kill-switch,
terminal audit row) is identical on the auto path; only the arming gate differs.

A runnable reference harness lives in [`examples/agent/`](../../examples/agent/)
(`wickd_agent.py`): a Python data→reason→orders loop that wraps the `wickd` binary,
delegates the reason step to `claude -p`, and **never passes `--live`**. Start with
the offline dry path, which prints the plan and the exact `wickd` commands without
running anything:

```sh
examples/agent/wickd_agent.py --explain
examples/agent/wickd_agent.py --instrument EUR_USD --strategy ma-crossover  # one real pass
examples/agent/wickd_agent.py --no-llm                                       # heuristic brain, no claude
```

The JSON contract the loop relies on is the CLI's standard one: each command
prints exactly one JSON object on stdout and exits 0; errors print
`{"error":{"code","message"}}` with a stable exit code (2 auth, 3 oanda,
4 validation). See `src/output.rs`.

## Scripted strategies

`strategy run` and `backtest` also accept a Rhai `.rhai` script in place of a
built-in strategy name — an explicit file path, or a bare name resolved under
`~/.wickd/strategies/<name>.rhai`. The full authoring contract (the
`on_candle()`/`on_position_closed()` functions, the `@indicators`/`@parameters`
metadata format, the ~18-function SDK, and the resource-safety limits every
script runs under) is documented and versioned in
[`STRATEGY_ABI.md`](./STRATEGY_ABI.md).

## `wickd audit` — append-only execution audit log

Every execution decision the CLI makes — **paper or live**, `place` or `close` —
is recorded as one immutable row in a local SQLite store at `~/.wickd/audit.db`.
The log is **append-only by construction**: the code has only insert and select
paths, with no `UPDATE` or `DELETE` anywhere. Rows, once written, are never
changed or removed.

On the **live** path the audit row is written *before* the order is submitted to
OANDA, and a failed write aborts the trade — so a live order can never reach the
broker without a ledger row already on disk. The post-response row (outcome:
`filled`/`rejected`/`no_fill`) is then appended fire-and-forget. Paper decisions
are a single fire-and-forget row.

Read recent decisions (newest first) as JSON:

```sh
wickd audit              # most recent 50
wickd audit --limit 200
```

Or query the raw store directly:

```sh
sqlite3 ~/.wickd/audit.db 'SELECT * FROM audit_log ORDER BY id DESC LIMIT 20'
```

Schema:

```sql
CREATE TABLE IF NOT EXISTS audit_log(
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  ts          TEXT NOT NULL,   -- RFC3339 timestamp
  instrument  TEXT,            -- e.g. EUR_USD
  units       INTEGER,         -- signed; negative = short
  sl          TEXT,            -- stop-loss price
  tp          TEXT,            -- take-profit price
  mode        TEXT NOT NULL,   -- 'paper' | 'live'
  environment TEXT,            -- 'practice' | 'live'
  action      TEXT NOT NULL,   -- 'place' | 'close'
  outcome     TEXT NOT NULL,   -- 'not_submitted' | 'attempt' | 'filled' | 'rejected' | 'no_fill'
  detail      TEXT             -- fill id, cancel reason, realized pl, side
);
```

## `wickd trade report` — account P&L vs a recorded baseline

Adjudicating a forward paper evaluation (see wickd-lab `PROTOCOL.md`) needs each
named account's performance against the balance it *started* with — a marker
OANDA does not keep. `wickd trade baseline` records that marker; `wickd trade
report` measures against it.

```bash
# Record where an account starts (once, per account). With no --balance, the
# account's current OANDA balance is fetched and used.
wickd trade baseline set --env practice --account h004 --balance 10000
wickd trade baseline set --account h004                 # fetch balance from OANDA
wickd trade baseline set --account h004 --date 2026-07-05   # backdate the marker

wickd trade baseline show    --account h004   # latest baseline
wickd trade baseline history --account h004   # every baseline, newest first

# Performance since the baseline.
wickd trade report --account h004
```

Baselines are stored durably in a local append-only SQLite store at
`~/.wickd/baselines.db`. Recording a new baseline **supersedes** the prior one
(the report reads the latest) but keeps every prior baseline in history — like
the audit log, the store is insert-only (no update/delete).

The report JSON carries:

- `realized_pl_since_baseline` — sum of the realized P&L of every trade **closed
  since the baseline date** (from OANDA's closed-trade history);
- `unrealized_pl` — OANDA's live unrealized P&L on open positions;
- `nav` and `nav_vs_baseline` — the account's current OANDA NAV and its change
  since the baseline balance;
- `by_strategy` — realized P&L grouped by strategy, using the AGT-630
  `clientExtensions` tag OANDA echoes onto each closed trade (manual/legacy
  trades fall under `"unattributed"`);
- `closed_trades` — the closed-trade list (instrument, units, open/close time,
  realized P&L, strategy).

**Reconciliation (sanity check).** The headline `nav` is OANDA's own account NAV,
so the report reconciles with the broker by construction. As an explicit check
the report also reconstructs NAV as `baseline + realized_pl_since_baseline +
unrealized_pl` and surfaces the `reconciliation.residual` between that and
OANDA's NAV. With no deposits/withdrawals and all trades inside the fetch window,
the residual is ~0 net of financing/fees; a large residual means funds moved or
some trades predate the baseline (raise `--limit` to widen the closed-trade
fetch, default 500).

## `wickd view` — on-demand GUI views

wickd is **headless by default**. The only code path that opens a window is
`wickd view <name>`, which mounts a [`@openthink/ui-leaf`](https://github.com/OpenThinkAi/ui-leaf)
view on demand and tears it down when you are done. There is no always-on GUI
and nothing background-launches a window.

The retired Tauri FX-ticket window is rebuilt as the `ticket` view; the
`watcher` view is a live monitor over the `wickd watch` signal daemon.

### Dependency

`wickd view` shells out to the `ui-leaf` binary, which must be on your `PATH`:

```sh
npm i -g @openthink/ui-leaf
```

If `ui-leaf` is missing, `wickd view` fails with a structured error
(`{"error":{"code":"ui_leaf_not_installed",...}}`, exit code 4) — it never
panics. Overrides:

- `WICKD_UI_LEAF_BIN` — path to the `ui-leaf` binary (default: `ui-leaf` on PATH).
- `WICKD_VIEWS_ROOT` — directory holding the view `.tsx` assets (default: the
  bundled `crates/wickd/views`, resolved relative to the binary or source tree).

### Launch

```sh
wickd view ticket                      # FX trade ticket, default EUR_USD
wickd view ticket --instrument GBP_USD # preload a different instrument
wickd view ticket --no-window          # mount without opening a browser (smoke test)
```

The command spawns `ui-leaf mount`, hands it the view spec over stdio, opens a
browser window with the FX ticket (instrument header, bid/ask/spread, buy/sell
order form), and **blocks** while the view is open. Live quotes and order
routing are not wired yet — this is a renderable skeleton.

### Teardown

Tear down the view in either of two ways:

1. **Close the browser window/tab.** wickd notices the disconnect and shuts the
   ui-leaf process down cleanly.
2. **Press Ctrl-C** in the terminal running `wickd view`.

Both send a graceful `{"version":"1","type":"close"}` to ui-leaf, wait for the
child to exit, print `{"view":"ticket","status":"closed"}`, and return. No GUI
process is left running.

## `wickd view watcher` — live signal monitor

`wickd view watcher` is a ui-leaf view over the **`wickd watch` signal daemon**
(see *Watch*): it renders the daemon's live NDJSON signals — pattern matches,
ticks, status, and errors — in a monitoring dashboard (status pill, tick/match
counters, last-close readout, and a newest-first signal log). It is **read-only**:
no orders are placed and the view registers no mutations. Like every `wickd
view`, it is **on-demand and headless** — nothing launches a window unless you
run this command.

### How it consumes the daemon signals (AC2)

The command reads the *exact* NDJSON the `wickd watch` daemon emits, folds each
line into a running view state, and pushes it to the browser via the ui-leaf
stdio protocol's `update` message, so the view re-renders on every signal. There
are two source modes, both genuinely consuming the daemon's stream:

- **Spawn (default):** `wickd view watcher <strategy> <instrument>` spawns
  `wickd watch <strategy> <instrument> …` as a child and forwards its stdout.
  Single-command UX; requires OANDA credentials just like `wickd watch`.
- **`--stdin`:** read the NDJSON from stdin, so you can pipe a daemon you already
  run. Credential-free at the view layer.

### Launch

```sh
# Spawn the daemon and monitor it in one command:
wickd view watcher ma-crossover EUR_USD --fast 10 --slow 30 --granularity H1
wickd view watcher rsi EUR_USD --period 14 --overbought 70 --oversold 30

# Or pipe an already-running daemon's stream in:
wickd watch ma-crossover EUR_USD | wickd view watcher ma-crossover EUR_USD --stdin

wickd view watcher ma-crossover EUR_USD --no-window   # mount without a browser (smoke test)
```

The strategy/instrument and strategy params (`--fast/--slow`, `--period/--overbought/--oversold`,
`--granularity/--count/--env`) mirror `wickd watch` and are forwarded to the
spawned daemon child.

### Teardown

Close the browser tab **or** press Ctrl-C. wickd sends the graceful
`{"version":"1","type":"close"}` to ui-leaf, **kills the spawned `wickd watch`
child** (if any), prints `{"view":"watcher","status":"closed"}`, and returns. If
the daemon's stream ends on its own (daemon exits / pipe closes), the view stays
open and flips its status to `stopped` — close it the same way. No GUI or daemon
process is left running.
