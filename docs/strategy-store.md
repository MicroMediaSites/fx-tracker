# The unified strategy store (`~/.wickd/strategies/`)

> Landed by **AGT-651**. One canonical strategy world for the wickd CLI and
> the desktop app: `.rhai` files in the CLI's data home, metadata comments as
> the single source of truth, the app as a read-only viewer/runner. The
> rules-JSON visual builder is gone — strategies are authored with Claude (or
> any editor) and enter through the CLI.

## What the store is

A directory of `.rhai` files — `~/.wickd/strategies/` — where **the
filesystem is the store**:

- A strategy's canonical name/id is its **file stem**
  (`revert_adx.rhai` → `revert_adx`).
- Its parameters and indicators live in the script's
  `@parameters` / `@indicators` metadata comments (see
  `crates/wickd/STRATEGY_ABI.md`) — there is no database column to drift
  out of sync.
- Only top-level `*.rhai` files are strategies; subdirectories (e.g. an
  `attic/` for parked scripts) are ignored.
- Names are slugs (`[A-Za-z0-9._-]`, no leading dot), so a name can never
  escape the store directory.

The implementation is `wickd_core::strategy_store::StrategyStore`
(`crates/wickd-core/src/strategy_store.rs`), shared by both hosts:

| Host | Surface |
|---|---|
| wickd CLI | `wickd strategy list/show/add/update/remove/convert`, plus bare-name resolution in `strategy run` / `backtest` / `watch` |
| Desktop app | `store_list_strategies` / `store_read_strategy` Tauri commands (read-only); the backtest window lists store strategies alongside archived local-store rows and runs them through the shared engine |

**Back-compat is structural:** the store's name resolution is byte-for-byte
the CLI's historical `~/.wickd/strategies/<name>.rhai` lookup, so every
pre-store file — including strategies a live watcher is holding open — keeps
resolving at its current path. Nothing moves, renames, or rewrites existing
files.

For tests and smoke runs, `WICKD_HOME=<dir>` overrides the data home
(`wickd_core::paths::wickd_data_home`); never set it for live daemons.

## CLI lifecycle

```bash
# List: built-ins + every store entry with parsed metadata + validity
wickd strategy list

# Validate a script (path or store name) — the agent-authoring surface
wickd strategy validate ./draft.rhai
wickd strategy validate revert_adx

# Add a new strategy (validates first; refuses to overwrite)
wickd strategy add ./draft.rhai --name mean_revert_v2
cat draft.rhai | wickd strategy add - --name mean_revert_v2

# Inspect: metadata + full source
wickd strategy show mean_revert_v2

# Update an existing strategy (validates first; name must exist)
wickd strategy update mean_revert_v2 ./draft-v3.rhai

# Remove
wickd strategy remove mean_revert_v2

# Run / backtest by store name (unchanged)
wickd strategy run mean_revert_v2 EUR_USD --granularity H1
wickd backtest mean_revert_v2 EUR_USD --granularity H1 --count 2000
```

All verbs emit JSON (the CLI's standard envelope). `add`/`update` refuse
invalid scripts — a script that doesn't pass `validate_script` never lands
in the store.

## Converting archived CandleSight rules strategies

`wickd strategy convert` translates rules-JSON `StrategyDefinition`s (the
retired visual-builder format) into ABI-dialect `.rhai` scripts,
implementing recipes R1–R6 from `docs/rhai-dialect-diff.md`:

```bash
# definitions.json: one StrategyDefinition or an array (e.g. extracted from
# the archive dump's strategy table)
wickd strategy convert definitions.json --out-dir ./converted
```

- `strategy_type: "scripted"` rows pass through verbatim (AGT-637 verified
  all archived scripted strategies validate under the current ABI as-is).
- Rules strategies are translated: compare/threshold/cross triggers,
  session + trend/ranging/volatility regime givens (auto-declaring the
  ADX/SMA20/SMA50/Bollinger indicator set), risk/reward + percent-of-TP +
  bar-count/time exits (exact via ABI v5 position state), and
  `risk_settings`-derived SL/TP arithmetic.
- Byte-identical duplicates (the archive has several) are deduped; the
  empty-shell strategy is dropped; unsupported constructs (multi-timeframe
  indicators, S/R-zone / pivot / pattern / divergence triggers, variables,
  partial closes) are **refused with a per-strategy reason** — those match
  the dialect report's "needs redesign" classification.
- Every emitted script is run through `validate_script` before it is
  written.

The output directory is explicit on purpose: review the converted scripts,
backtest them, then install the keepers with `wickd strategy add`.

> **Live-watcher guard:** while a pinned watcher (H-004) is running, do NOT
> bulk-write converted output straight into `~/.wickd/strategies/`. Convert
> into a scratch dir, review, and `wickd strategy add` the keepers — adds are
> new files and never touch the scripts a live watcher has loaded.

## The desktop app is a viewer/runner

The visual rules builder and its editing paths were deleted (AGT-651 AC2).
The backtest window now:

- lists store strategies (badged `.rhai`, read from `~/.wickd/strategies/`)
  alongside archived local-store rows (AGT-648's namespaced imports),
- shows strategy source read-only,
- runs backtests against them through the same
  `wickd_core` engine and host constructor the CLI uses
  (`ScriptedStrategy::for_host`), so app runs and CLI runs see identical
  event-calendar/surprise/pip wiring (dialect report D1–D3 are structurally
  closed).

Local-store `strategy` rows remain for archived/imported data (see
`docs/local-store.md` v1) but the app no longer creates or edits strategies.

## One engine, one constructor

`ScriptedStrategy::for_host(script, name, overrides, instrument)`
(wickd-core) is the single construction path for every host: it resolves
parameters, sets the instrument pip value, and injects the event calendar
(`~/.wickd/events.json`, else the bundled schedule) and surprise feed
(`~/.wickd/calendar/*.csv`). The calendar loaders moved from the CLI into
`wickd_core::events` so `src-tauri` wires exactly the same feeds. If you add
host wiring, add it inside `for_host` — not in a host.
