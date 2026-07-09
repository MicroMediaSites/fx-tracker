# wickd local app store (`~/.wickd/app.db`)

> Foundation landed by **AGT-642**. This is the local-first SQLite store the
> CandleSight→wickd conversion builds on; the follow-up domain migrations
> (AGT-645/646/647) extend it rather than inventing new persistence.

## What it is

A single SQLite database at **`~/.wickd/app.db`** — the same data home the
wickd CLI already uses for its stores (`audit.db`, `baselines.db`,
`alerts.json`, ...). It is owned by the Rust side of the desktop app and holds
everything the app can serve **offline, with no sign-in**.

The default cold-boot window (`?window=local`, label `main`) renders entirely
from this store. Since AGT-650 there is no Zero context anywhere: every app
window (local/chart/backtest/watcher) mounts without auth and reads this
store (plus OANDA via Tauri commands). The Clerk sign-in shell and every
other auth surface were deleted (AGT-652/AGT-653) — the local credentials
vault is the only credential store.

## Layers (where code goes)

| Layer | Location | Role |
|---|---|---|
| Schema / migrations | `src-tauri/src/local_store/migrations.rs` | **The** home for local schema. Append a new entry to `MIGRATIONS`; never edit or reorder existing ones. Versioned via `PRAGMA user_version`. |
| Data-access layer | `src-tauri/src/local_store/mod.rs` | All SQL. One `LocalStore` handle; add new dataset methods here. `open_at(path)` exists so tests never touch the real store. |
| Tauri commands | `src-tauri/src/commands/local_store.rs` | Thin invoke-wrappers (`local_*`). Store opens lazily on first command; open failure is a command error, never a boot crash. |
| Frontend API | `src/lib/localStore.ts` | The only place the frontend calls the store. Typed wrappers + row types. |
| E2E stand-in | `e2e/mocks/tauri-bridge.ts` (`window.__E2E_LOCAL_STRATEGIES__`, `__E2E_LOCAL_SR_ZONES__`, `__E2E_LOCAL_NOTES__`, `__E2E_LOCAL_CHART_CONFIGS__`, `__E2E_LOCAL_TRADES__`, `__E2E_LOCAL_TRADE_SCORES__`, `__E2E_LOCAL_BACKTESTS__`, `__E2E_LOCAL_JOBS__`, `__E2E_LOCAL_PROMOTIONS__`, `__E2E_LOCAL_LABELS__`, `__E2E_LOCAL_TRADE_LABELS__`, `__E2E_LOCAL_STRATEGY_LABELS__`, `__E2E_LOCAL_STRATEGY_TRADES__`, `__E2E_LOCAL_STRATEGY_WATCHERS__`, `__E2E_LOCAL_CREDENTIAL__`) | Stateful in-memory mock; pre-seed with `addInitScript` before navigation (or via the `appPage.setLocalStrategies` / `setLocalBacktests` / `setLocalDataset` fixture helpers). |

> The old rule "all migrations in `queries-service/src/migrate.ts`" applied
> to the retired cloud (Zero/Postgres) path, which AGT-650 deleted outright
> (queries-service/, `src-tauri/src/db.rs`, the Zero schema, and their CI
> guards are all gone). Local schema lives in `local_store/migrations.rs`,
> full stop.

## Conventions

- **Row shapes mirror the Zero schema rows** (snake_case fields, JSON-encoded
  sub-objects as `TEXT` strings, epoch-**millisecond** integer timestamps,
  booleans as 0/1) so components can switch data sources without remapping and
  data migrations are straight copies.
- **No `user_id`** — the local store is single-user by design (wickd is
  personal-first; see `wickd-architecture.md` in the vault).
- **Bundled rusqlite** (`features = ["bundled"]`), WAL journal mode.
- Money stays `TEXT`/`Decimal`-safe when trades arrive (AGT-645): never store
  prices as REAL.

## Current datasets

| Version | Dataset | Notes |
|---|---|---|
| v1 | `strategy` | Walking skeleton (AGT-642). Columns = Zero `strategy` table minus `user_id`. |
| v2 | `sr_zone`, `note`, `chart_config` | Charting domain (AGT-646). `sr_zone`/`note` = Zero tables minus `user_id`; `chart_config` keys per-instrument chart indicator JSON (replaces the old localStorage persistence, one-time import on first read). |
| v3 | `trade`, `trade_score` | Trades/analysis domain (AGT-647). Columns = Zero tables minus `user_id`; trade ids are raw OANDA ids (no `userID:oandaId` composites). `trade_score.trade_id` is UNIQUE — one AI score per trade. |
| v4 | `backtest`, `backtest_job`, `promotion_audit` | Strategies + backtests domain (AGT-645). Columns = the Zero tables minus `user_id`. `backtest.results` carries the **full** run payload (metrics + trades + equity curve + parameter snapshot) so the UI renders runs offline. |
| v5 | `source` column on every dataset table | Provenance tag (AGT-648): `''` = native wickd data, `'candlesight'` = restored from the CandleSight archive by the `import_candlesight` CLI (`src-tauri/src/bin/`, logic in `local_store/import.rs`; see `docs/candlesight-archive.md`). Exposed on `LocalStrategy` for the local window's badge/filter; other datasets keep it storage-only for now. |
| v6 | `label`, `trade_label`, `strategy_label`, `strategy_trade`, `strategy_watcher`, `credential` | Zero-removal sweep (AGT-650): the last Zero-backed domains. Labels + junctions (TradingTicketApp/LabelPicker/LabelSelector), strategy↔OANDA-trade attribution (useTradeExecution/useStrategyTrades), persisted watcher configs (StrategyWatcherApp auto-start), and the device's encrypted OANDA credential blobs (moved from the cloud `user_credentials` table; ciphertext only — the Rust crypto vault still does all encrypt/decrypt). Data tables carry the AGT-648 `source` column; `credential` is device-local and never archived. |

## Command surface

| Command | Signature | Notes |
|---|---|---|
| `local_store_path` | `() -> String` | Absolute DB path for display/diagnostics. |
| `local_list_strategies` | `() -> Vec<LocalStrategy>` | `updated_at DESC`, includes archived/inactive (filtering is a UI concern). |
| `local_get_strategy` | `(id) -> Option<LocalStrategy>` | |
| `local_save_strategy` | `(strategy) -> ()` | Upsert keyed on `id`. |
| `local_delete_strategy` | `(id) -> bool` | Hard delete; use `is_active=false` via save for tombstoning. |
| `local_list_sr_zones` | `(instrument?) -> Vec<LocalSRZone>` | `created_at ASC`. Omit instrument for all zones (watcher trigger maps). |
| `local_save_sr_zone` | `(zone) -> ()` | Upsert keyed on `id`. |
| `local_delete_sr_zone` | `(id) -> bool` | |
| `local_clear_sr_zones` | `(instrument) -> usize` | "Clear all" for one chart. |
| `local_list_notes` | `(trade_id?, strategy_id?) -> Vec<LocalNote>` | `created_at DESC`. No filters = all notes (note-count badges). |
| `local_save_note` | `(note) -> ()` | Upsert keyed on `id`. |
| `local_delete_note` | `(id) -> bool` | |
| `local_get_chart_config` | `(instrument) -> Option<String>` | JSON `ChartIndicatorConfig[]`. |
| `local_set_chart_config` | `(instrument, indicators) -> ()` | Upsert keyed on `instrument`. |
| `local_list_trades` | `() -> Vec<LocalTrade>` | `open_time DESC`, open + closed. |
| `local_list_closed_trades_by_instrument` | `(instrument) -> Vec<LocalTrade>` | Chart trade-marker overlay read path. |
| `local_list_trade_scores` | `() -> Vec<LocalTradeScore>` | Badges scored trades in the analysis list. |
| `local_get_trade_score_by_trade` | `(trade_id) -> Option<LocalTradeScore>` | |
| `local_save_trade_score` | `(score) -> ()` | Upsert keyed on `trade_id` (one score per trade). |

Trades have **no frontend write command**: the Rust `sync_trades` command
(`commands/data.rs`) fetches from OANDA and upserts straight into the store
via `LocalStore::upsert_trades` — no queries-service, no auth token. It opens
its own store handle inside the background task (WAL keeps that cheap).

| Command | Signature | Notes |
|---|---|---|
| `local_list_backtests` | `(strategy_id) -> Vec<LocalBacktest>` | `created_at ASC` (run order). |
| `local_save_backtest` | `(backtest) -> ()` | Upsert keyed on `id`. |
| `local_delete_backtests_for_strategy` | `(strategy_id) -> usize` | Bulk delete (the backtest UI's "Reset"). |
| `local_list_backtest_jobs` | `(strategy_id) -> Vec<LocalBacktestJob>` | `updated_at DESC`. |
| `local_get_backtest_job` | `(id) -> Option<LocalBacktestJob>` | |
| `local_save_backtest_job` | `(job) -> ()` | Upsert keyed on `id`; the frontend persists start/heartbeat/completion. |
| `local_record_promotion` | `(audit) -> ()` | Append-only promotion/demotion audit row. |

AGT-650 (labels / attribution / watcher configs / credential):

| Command | Signature | Notes |
|---|---|---|
| `local_list_labels` | `() -> Vec<LocalLabel>` | Alphabetical. |
| `local_save_label` | `(label) -> ()` | Upsert keyed on `id`. |
| `local_list_trade_labels` | `(trade_id?) -> Vec<LocalTradeLabel>` | Omit for all junctions. |
| `local_add_trade_label` | `(trade_label) -> ()` | |
| `local_delete_trade_label` | `(id) -> bool` | Detach by junction id. |
| `local_list_strategy_labels` | `(strategy_id?) -> Vec<LocalStrategyLabel>` | |
| `local_add_strategy_label` | `(strategy_label) -> ()` | |
| `local_delete_strategy_label` | `(id) -> bool` | |
| `local_list_strategy_trades` | `(strategy_id?) -> Vec<LocalStrategyTrade>` | `executed_at DESC`. |
| `local_save_strategy_trade` | `(strategy_trade) -> ()` | Upsert keyed on `id`. |
| `local_list_strategy_watchers` | `() -> Vec<LocalStrategyWatcher>` | `updated_at DESC`; includes inactive rows (signal-filter memory, BUG-015). |
| `local_save_strategy_watcher` | `(watcher) -> ()` | Full-row upsert keyed on `id` (config id `strategyId-instrument-timeframe`). |
| `local_delete_strategy_watcher` | `(id) -> bool` | |
| `local_get_credential` | `() -> Option<LocalCredential>` | Single-row semantics; `None` before onboarding. |
| `local_save_credential` | `(credential) -> ()` | Ciphertext blobs only. |
| `local_delete_credentials` | `() -> usize` | The "reset credentials" flow. |

## Frontend consumers (AGT-645)

The backtest window's strategies + backtests domain is fully local:
`useParsedStrategies` (reads), `useStrategyMutations` / `useStrategyPromotion`
(writes, via the read-modify-write `updateStrategy` helper in
`src/lib/localStore.ts`), `useBacktestJob` (job rows), and
`SimpleHistoricalFlow` (persists + rehydrates saved runs). The offline local
window renders saved runs via `src/components/local/LocalBacktestsSection.tsx`.
Partial updates MUST go through `updateStrategy`/`updateBacktestJob` — the raw
save commands are full-row upserts.

## Adding a dataset (the AGT-645/646/647 recipe)

1. Append one `CREATE TABLE ...` entry to `MIGRATIONS` in
   `local_store/migrations.rs` (new version).
2. Add the row struct + DAL methods in `local_store/mod.rs`, with unit tests
   against `LocalStore::open_at(temp_path)`.
3. Add `local_*` commands in `commands/local_store.rs`; register them in
   `main.rs`'s `generate_handler!`.
4. Add typed wrappers in `src/lib/localStore.ts`.
5. Add default handlers to `e2e/mocks/tauri-bridge.ts` and cover the flow in
   an E2E spec.

## MCP consumer (AGT-649)

`mcp-server-rs` (the `wickd-mcp` binary) is a **second process** reading the
same store: a local stdio MCP server for Claude sessions (Claude Desktop /
Claude Code / claude.ai desktop connectors). It replaced the Railway
`production-mcp` HTTP deployment — no OAuth, no Postgres, no deploy label.

- **Schema sharing:** the crate includes `src-tauri/src/local_store/migrations.rs`
  verbatim via `#[path]` (`mcp-server-rs/src/store.rs`), so there is exactly one
  migration list. It applies migrations idempotently on open, same as the app.
- **Path resolution:** `~/.wickd/app.db` by default; `WICKD_DB_PATH` overrides
  (tests and smoke runs use temp stores, never the live one).
- **Concurrency:** WAL mode means the app and `wickd-mcp` can read/write
  concurrently without stepping on each other.
- **Tool surface:** strategies (list/get/create/update), trades + account
  summary, notes, S/R zones, backtest results, help topics, and AI strategy
  conversion (`ANTHROPIC_API_KEY`). Pattern-match and calendar tools died with
  the cloud tables — they had no local dataset.
