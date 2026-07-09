# Claude Code Instructions for wickd

wickd is a **local-first desktop trading app** (Tauri 2 + React 19) that fronts
the wickd daemon. There is no cloud tier: no auth service, no billing, no sync
backend. Everything reads the local SQLite store (`~/.wickd/app.db`) or talks
to OANDA directly via Tauri commands. The CandleSight→wickd conversion
(AGT-635..653) deleted the old Zero/Clerk/queries-service/Stripe/marketing
architecture — if you find docs or comments describing it, they are historical.

## Project Structure

- **Frontend**: React 19 in `src/` — windows: local (default boot), chart,
  backtest, watcher (`?window=` URL param, see `src/index.tsx`)
- **Backend**: Rust + Tauri 2 in `src-tauri/`
- **Shared crates**: `crates/` (wickd-core — also used by the wickd CLI/daemon)
- **MCP server**: `mcp-server-rs/` — local stdio binary (`wickd-mcp`) reading
  `~/.wickd/app.db`
- **E2E**: `e2e/` (Playwright against Vite dev server with mocked Tauri IPC)

## Key Files

- `src-tauri/tauri.conf.json` - Tauri window config, CSP, updater
- `src-tauri/src/local_store/` - local SQLite store (schema + migrations)
- `vite.config.ts` / `vite.config.e2e.ts` - Vite config (app / e2e mocks)

## Development

```bash
npm run start:app    # Tauri dev app
npm run check        # TypeScript check
```

- **User runs their own dev server** - Do NOT leave background processes running
- If you need to start a dev server to test something, kill it immediately after checking output
- Always clean up ports 1420/4848 if you used them

## Testing

- Backend tests: `npm run test:be` (or `cd src-tauri && cargo test`)
- Frontend tests: `npm run test:ui`
- Both: `npm test`
- E2E: `npm run test:e2e` (CI mode: `CI=1 npm run test:e2e`)
- `cargo test --workspace` if you touch `crates/`
- All tests must pass before considering work complete

### E2E Tests (Playwright)

E2E tests run against the Vite dev server with mocked backends (no Tauri build
needed). Tauri IPC is mocked via `e2e/mocks/tauri-bridge.ts`; use the fixture
in `e2e/helpers/app-fixture.ts` (`appPage`) and
`appPage.mockTauriCommand(name, response)` for per-test overrides. The offline
boot specs (`e2e/tests/local-mode-offline-boot.spec.ts`,
`e2e/tests/agt-650-offline-app-windows.spec.ts`) assert the app makes zero
non-localhost requests — keep them green.

## Database Migrations

Local schema migrations live ONLY in `src-tauri/src/local_store/migrations.rs`
(append-only: add a new entry to `MIGRATIONS`, never edit or reorder existing
ones; versioned via `PRAGMA user_version`). See `docs/local-store.md`.

## Coding Patterns

### Frontend (React)
- Local data via Tauri commands (`@tauri-apps/api/core` invoke) and zustand stores
- Streaming via `@tauri-apps/api/event` listen
- Reusable UI primitives go in `src/components/ui/`
- Destructive actions require confirmation modals - never single-click delete

### Backend (Rust)
- **ALWAYS use `rust_decimal::Decimal`** for prices/amounts/P&L - never f64
- Use atomic `compare_exchange` for thread-safe state checks (not check-then-set)
- Use type-safe enums over magic strings for order types, states, etc.
- Consolidate error handling patterns into helper functions
- Ensure state cleanup on ALL error paths
- Set TLS 1.2 minimum for HTTP clients

### Event Flow (Streaming)
- Backend emits events via `app_handle.emit("event-name", payload)`
- Frontend listens via `listen<T>("event-name", callback)`
- Error events: `stream-error` with `{ errorType, message }`
- Price events: `price-update` with `{ instrument, bid, ask, spread, time }`

### OANDA Candle Alignment
- Candles use `dailyAlignment=2` with `alignmentTimezone=UTC` (configured in the OANDA endpoints module)
- This gives H4 candles at 02:00, 06:00, 10:00, 14:00, 18:00, 22:00 UTC
- Matches OANDA's platform candle boundaries
- All candle fetching (charting, backtesting, strategy watcher) uses these settings
- Client-side boundary detection and `candle_boundary.rs` must stay in sync

## Git & Review

- This is a **stamp-protected repo** — see AGENTS.md for the required
  `stamp review` / `stamp merge` flow. No direct commits to main.

## NEVER Touch

- **Environment files**: NEVER read, write, edit, copy, move, or modify any `.env*` files
- **Builds**: NEVER run `npm run tauri build` or any production build script
- **Railway**: NEVER run `railway` CLI commands (the remaining Railway project is being torn down)
- **The running daemon**: never disturb `com.openthink.wickd-*` launchd jobs or `~/.wickd` state

<!-- stamp:begin (managed by `stamp init` — do not edit between markers) -->

## Stamp-protected repository — read AGENTS.md before any git operation

This repository is gated by [stamp-cli](https://github.com/OpenThinkAi/stamp-cli).
**Do not `git commit` directly to protected branches** (typically `main`)
**and do not `git push origin main`** of any commit you didn't produce via
`stamp merge`. The required flow is:

```sh
git checkout -b feature
# ... edit, commit on the feature branch ...
stamp review --diff main..feature       # all reviewers run in parallel
stamp status --diff main..feature       # gate check (exit 0 = open)
git checkout main
stamp merge feature --into main         # signs the merge
git push origin main                    # OR `stamp push main` if origin is a stamp server
```

Key commands: `stamp provision` — provision a new repo; `stamp review` — run reviewers; `stamp merge` — sign a merge; `stamp push` — push to a stamp server.

**The full reference is at [`AGENTS.md`](./AGENTS.md) at the repo root** —
read it before any git command. It covers the mode (server-gated vs.
local-only), what NOT to do, where things live, and how to recover when stamp
blocks you.

**One exception:** the very first commit that ADDS `.stamp/` + `AGENTS.md` +
`CLAUDE.md` to a fresh repo is allowed to land directly on the current branch
(there's nothing to review against). Recent `stamp init` runs do this commit
automatically. Every subsequent change goes through the stamp flow.

<!-- stamp:end -->
