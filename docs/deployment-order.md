# Deployment Order Guide

This document explains the deployment dependencies between wickd services and how to determine the correct deployment order for any PR.

## Service Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                         PostgreSQL (Railway)                        │
│                      Source of Truth Database                       │
└──────────────┬──────────────────┬─────────────────┬────────────────┘
               │                  │                 │
               ▼                  ▼                 ▼
    ┌──────────────────┐  ┌─────────────┐  ┌──────────────────┐
    │   zero-cache     │  │  queries-   │  │   mcp-server-rs  │
    │  (Zero sync)     │  │   service   │  │  (MCP tools)     │
    │                  │  │             │  │                  │
    │ Reads: schema.ts │  │ Reads:      │  │ Direct sqlx      │
    │ Syncs to clients │  │ schema.ts   │  │ queries          │
    └────────┬─────────┘  │ Mutations   │  │ Own DB pool      │
             │            │ AI proxy    │  └──────────────────┘
             │            └──────┬──────┘          │
             │                   │                 │
             ▼                   ▼                 │
    ┌────────────────────────────────────┐        │
    │         Tauri Desktop App          │        │
    │  (src-tauri/ + src/ frontend)      │◄───────┘
    │                                    │   (Claude Desktop
    │  - Zero sync for reads             │    uses MCP server)
    │  - queries-service for mutations   │
    │  - OANDA for trading               │
    └────────────────────────────────────┘
```

> **Post-AGT-649:** `mcp-server-rs` no longer deploys anywhere. `wickd-mcp` is
> a **local stdio MCP server** that reads the local store (`~/.wickd/app.db`)
> directly — it has no Railway service, no Postgres pool, and no deploy label.
> The diagram above shows the retired cloud topology for the remaining services.

## Service Details

### 1. PostgreSQL (Railway)
- **Location**: Railway managed database
- **Role**: Source of truth for all data
- **Migrations**: Run via `src-tauri/src/db.rs` on app startup
- **Deploy trigger**: Migrations run when Tauri app connects

### 2. zero-cache (Railway)
- **Location**: `Dockerfile`, `entrypoint.sh`, `shared/schema.ts`
- **Role**: Real-time sync between PostgreSQL and clients via Zero protocol
- **Schema source**: `shared/schema.ts` → compiled to `schema.cjs`
- **Deploy trigger**: `deploy:zero` label
- **Dependencies**: Database schema must exist first

### 3. queries-service (Railway)
- **Location**: `queries-service/`
- **Role**: Handles Zero mutations, AI proxy, subscription management
- **Schema source**: `queries-service/schema.ts` (manually synced from shared)
- **Deploy trigger**: `deploy:queries` label
- **Dependencies**: Database schema, Zero schema sync

### 4. mcp-server-rs (local — no deployment)
- **Location**: `mcp-server-rs/`
- **Role**: MCP stdio server for Claude integration (`wickd-mcp` binary)
- **Database access**: rusqlite against the local store `~/.wickd/app.db`
  (schema shared with the app via `src-tauri/src/local_store/migrations.rs`)
- **Deploy trigger**: none — build locally (`cargo build --release`) and point
  the MCP client config at the binary. The Railway `production-mcp` /
  `staging-mcp` services are retired (AGT-649).

### 5. Tauri Desktop App (GitHub Release)
- **Location**: `src-tauri/`, `src/`
- **Role**: Desktop application users install
- **Deploy trigger**: production releases are cut **automatically on every push
  to `main`** (stamp merge → mirror push → `.github/workflows/release.yml`).
  The version bump is computed from conventional-commit messages since the last
  `v*` tag (`type!:`/`BREAKING CHANGE` → major, `feat:` → minor, anything else
  → patch); the git tag is the source of truth for the released version.
  Staging builds still use the `build:staging` label.
- **Dependencies**: ALL backend services must be ready first

### 6. Marketing Site (Netlify)
- **Location**: `web/`
- **Role**: Marketing website
- **Deploy trigger**: `deploy:web` label
- **Dependencies**: None (static content)

---

## Deployment Order Rules

### Rule 1: Database Schema First
If a PR adds/modifies database columns:
1. The migration runs when the Tauri app starts (in `db.rs`)
2. BUT zero-cache and queries-service need the schema definition
3. **Order**: Deploy `deploy:zero` + `deploy:queries` BEFORE merging app
   changes to `main` (merging to `main` auto-cuts a production release)

### Rule 2: Backend Before Frontend
If a PR adds new queries, mutations, or API endpoints that the app uses:
1. Backend services must have the code BEFORE the app ships
2. **Order**: backend `deploy:*` changes merge and deploy first; app changes
   merge to `main` afterwards (the merge itself cuts the release)

> **Note (post-AGT-641):** scenarios below that say "add `build:production`"
> reflect the retired label flow. Read them as "merge the app change to
> `main`" — the release is cut automatically by the merge.

### Rule 3: Schema Sync Across Services
If a PR modifies `shared/schema.ts`:
1. `queries-service/schema.ts` must be manually synced
2. `schema.cjs` must be regenerated (`npm run predev`)
3. **Order**: `deploy:zero` + `deploy:queries` together

### Rule 4: MCP is Local
The MCP server reads the local store directly and has no deployment at all
(AGT-649). Changes under `mcp-server-rs/` ship by rebuilding the local binary;
if they depend on a new local-store dataset, the migration in
`src-tauri/src/local_store/migrations.rs` lands in the same repo — no ordering
concern beyond merging.

---

## Common Scenarios

### Scenario A: New Database Column + Frontend Display
**Example**: Add `profit_factor` column to backtests, display in app

**Files changed**:
- `src-tauri/src/db.rs` - Migration
- `shared/schema.ts` - Zero schema
- `queries-service/schema.ts` - Schema sync
- `src/components/BacktestResults.tsx` - Display it

**Correct labels**: `deploy:zero`, `deploy:queries`
**DO NOT add**: `build:production` (wait for backend first)

**Deployment order**:
1. Merge PR with `deploy:zero` + `deploy:queries`
2. Wait for Railway deploy to complete
3. Create separate PR or manual action for `build:production`

---

### Scenario B: New Query in queries-service + Frontend Usage
**Example**: Add endpoint to get strategy statistics

**Files changed**:
- `queries-service/src/routes.ts` - New endpoint
- `src/hooks/useStrategyStats.ts` - Frontend hook

**Correct labels**: `deploy:queries`
**DO NOT add**: `build:production`

**Deployment order**:
1. Merge PR with `deploy:queries`
2. Verify endpoint works in staging
3. Merge app changes with `build:production`

---

### Scenario C: MCP Server + Strategy Schema Changes
**Example**: Add new indicator type to strategies

**Files changed**:
- `shared/src/strategy.rs` - Rust types
- `mcp-server-rs/src/main.rs` - MCP validation
- `src-tauri/src/backtest/` - Backtest engine
- `src/components/StrategyBuilder.tsx` - UI

**Correct labels**: `build:staging`
**Why**: MCP server validates strategies with the shared Rust types, but it is
a local binary now — rebuild `wickd-mcp` after merge, no deploy label.

**Deployment order**:
1. Merge; rebuild the local `wickd-mcp` binary (picks up new validation)
2. Deploy `build:staging` (test app with new types)
3. Later: `build:production`

---

### Scenario D: Frontend-Only Change
**Example**: Fix UI bug, add tooltip, change styling

**Files changed**:
- `src/components/*.tsx` only

**Correct labels**: `build:staging` + `bump:patch`
**No backend labels needed**

---

### Scenario E: Backend-Only Change
**Example**: Fix bug in queries-service, improve AI proxy

**Files changed**:
- `queries-service/src/*.ts` only

**Correct labels**: `deploy:queries`
**No build labels needed** (unless you want to test in app)

---

### Scenario F: Breaking API Change (Staged Rollout)
**Example**: Rename mutation parameter from `strategyId` to `strategy_id`

**This requires THREE steps**:

1. **First PR**: Backend accepts BOTH formats
   - Labels: `deploy:queries`
   - queries-service accepts `strategyId` OR `strategy_id`

2. **Second PR**: App uses new format
   - Labels: `build:production`
   - App sends `strategy_id`

3. **Third PR**: Backend removes old format
   - Labels: `deploy:queries`
   - queries-service only accepts `strategy_id`

---

## Label Reference

### Build Labels (Tauri App)
| Label | Triggers | When to Use |
|-------|----------|-------------|
| `build:staging` | Staging build only | Testing app changes |

**Production releases are no longer label-driven.** Every push to `main`
(i.e. every stamp merge, mirrored to GitHub) automatically cuts a signed
GitHub release; the version bump comes from conventional-commit messages
(`type!:`/`BREAKING CHANGE` → major, `feat:` → minor, else → patch). The
`build:production` and `bump:*` release labels are retired — write
conventional commits instead.

### Deploy Labels (Railway Services)
| Label | Service | Auto-deploys |
|-------|---------|--------------|
| `deploy:queries` | queries-service | Staging → Prod |
| `deploy:zero` | zero-cache | Staging → Prod |
| `deploy:web` | Marketing site | Netlify |
| `deploy:staging-only` | All | Stops at staging |

---

## PR Review Checklist for Deployment

When reviewing a PR, verify:

1. **Files changed** → Which services are affected?
2. **Dependencies** → Does the app need backend changes first?
3. **Labels present** → Are the right deploy/build labels applied?
4. **Labels absent** → Should `build:production` be withheld?
5. **Order documented** → For complex PRs, is deployment order in PR description?

### Red Flags (Block Merge)

- **`build:*` without `bump:*`** - Users won't receive the update
- `build:production` with new queries-service endpoint (backend not deployed yet)
- `deploy:zero` without `deploy:queries` when schema changed
- Missing `queries-service/schema.ts` sync when `shared/schema.ts` changed
- New database column used in app without migration in `db.rs`

### Yellow Flags (Verify Intent)

- `build:staging` without deploy labels (frontend-only?)
- Multiple deploy labels (coordinated release?)
- `deploy:staging-only` (manual promotion later?)
