# Data Infrastructure Architecture

## Zero Sync Architecture

The data layer uses Zero by Rocicorp for real-time sync between PostgreSQL and frontend clients. The architecture has three tiers:

```
Frontend (React)
    |
    v  WebSocket
zero-cache (Railway)
    |
    +---> queries-service (Railway) ---> PostgreSQL
    |         |-- PushProcessor (mutations)
    |         |-- handleGetQueriesRequest (queries)
    |         |-- Direct SQL (trade sync, jobs, spread stats)
    |
    +---> PostgreSQL (Railway)
              |-- WAL replication to zero-cache
```

**Why Zero?** Traditional REST requires polling or manual cache invalidation. Zero provides automatic real-time sync -- when PostgreSQL changes, all connected clients see the update within seconds via WebSocket. This is critical for a trading app where stale data means missed opportunities.

**Why zero-cache as a separate service?** Zero-cache maintains a SQLite replica of PostgreSQL via WAL (Write-Ahead Log) replication. It serves as a sync gateway -- clients connect to zero-cache via WebSocket, and zero-cache handles the delta compression, conflict resolution, and offline support. Keeping it separate from queries-service allows independent scaling.

## The Mutation Pipeline

Mutations flow through Zero's `PushProcessor` which provides transactional consistency:

```
Frontend                  zero-cache              queries-service
   |                          |                         |
   | zero.mutate.X.insert()   |                         |
   |  (optimistic update)     |                         |
   |------------------------->|  POST /push             |
   |                          |------------------------>|
   |                          |   1. Verify JWT         |
   |                          |   2. createMutators()   |
   |                          |   3. pushProcessor.process()
   |                          |      a. requireAuth()   |
   |                          |      b. verifyOwnership()|
   |                          |      c. validateFields() |
   |                          |      d. tx.mutate.X()   |
   |                          |<------------------------|
   |                          |   PostgreSQL commit      |
   |<-------------------------|                         |
   |  (sync confirmation)     |                         |
```

**Key security invariant:** The frontend `src/mutators.ts` contains optimistic mutations with NO security checks -- they just call `tx.mutate.X.insert(args)` directly. ALL security enforcement happens server-side in `queries-service/src/mutators.ts`:

1. **`requireAuth()`** -- Extracts userID from `AuthData` (derived from JWT). Throws if unauthenticated.
2. **`verifyOwnership()`** -- On server (`tx.location === 'server'`), does a direct SQL `SELECT` to verify the record's `user_id` matches the authenticated user. On client, this is a no-op (optimistic trust).
3. **`requireTier()`** -- Queries the `subscription` table to verify the user has the required tier (free/premium/pro). Client-side assumes access; server rejects if insufficient.
4. **Field validation** -- Zod schemas in `validation.ts` validate JSON fields (indicators, entry_rules, risk_settings), decimal strings, instrument formats, and timeframes.
5. **user_id forcing** -- On insert, the server ALWAYS overrides `user_id` from auth context: `{ ...args, user_id: userID }`. Never trusts client-supplied user_id.

**Why optimistic on client?** Zero's architecture requires client-side mutators to run immediately for responsive UI. The server then validates and can reject -- Zero automatically rolls back the optimistic update if the server push fails. This gives instant feedback while maintaining security.

### Push Endpoint Auth Flow

The `/push` endpoint returns 401 on ANY auth failure. This is critical because Zero's client intercepts 401 responses and calls the auth function with `'invalid-token'`, triggering an automatic token refresh and retry. Without this, expired Clerk tokens (60-second lifetime) would cause permanent mutation failures.

## The Query Pipeline

Queries use Zero's `syncedQueryWithContext` pattern:

```
Frontend                      zero-cache              queries-service
   |                              |                         |
   | useQuery(myTrades(user.id))  |                         |
   |----------------------------->|  POST /get-queries      |
   |                              |------------------------>|
   |                              |   1. extractUserID()    |
   |                              |   2. getQuery(name, args)
   |                              |   3. withValidation()   |
   |                              |<------------------------|
   |                              |   Query definition      |
   |<-----------------------------|                         |
   |  (real-time sync established)|                         |
```

### `syncedQueryWithContext` vs `syncedQuery`

- **`syncedQueryWithContext`** -- First parameter is `userID: string | undefined`. Used for all user-scoped data. The userID comes from the JWT `sub` claim on the server side, and from `user?.id` on the client side (for optimistic rendering).
- **`syncedQuery`** -- No user context. Currently unused -- even global data like `calendar_event` and `spread_stats` use `syncedQueryWithContext` (they just ignore the userID parameter).

**Critical:** The context parameter is `string | undefined`, NOT an object. This is a Zero SDK constraint.

### The `___never_match___` Pattern

When `userID` is undefined (user not yet authenticated), queries must still return a valid Zero query object. The pattern is:

```typescript
if (!userID) return builder.trade.where('id', '___never_match___');
```

This creates a valid query that matches zero rows. Why not return an empty result directly? Because Zero requires a query object to set up the subscription -- it needs to know WHAT to watch for changes.

## Migration Architecture

**SINGLE SOURCE OF TRUTH: `queries-service/src/migrate.ts`**

All `CREATE TABLE`, `ALTER TABLE`, `ADD COLUMN`, and `CREATE INDEX` statements live here and nowhere else. CI will fail if migrations are added to `src-tauri/src/db.rs` or any other file.

**Why?** Migrations run when queries-service deploys to Railway. This ensures staging and production databases stay in sync automatically. If migrations were in the Tauri desktop app, they would only run when a user opens the app -- production DB would fall behind.

### Migration Execution

```
queries-service startup
    |
    v
runMigrations(databaseUrl)
    |-- Opens a single connection (max: 1)
    |-- Runs all CREATE TABLE IF NOT EXISTS
    |-- Runs all ALTER TABLE ADD COLUMN IF NOT EXISTS
    |-- Runs all CREATE INDEX IF NOT EXISTS
    |-- Catches errors individually (logs but continues)
    |-- Closes connection
    |
    v
Server starts listening
```

Migrations are **idempotent** -- every statement uses `IF NOT EXISTS` or `IF EXISTS` guards and wraps in `.catch(() => {})`. Safe to run on every deploy.

### Deployment Order for Schema Changes

1. **Deploy queries-service** -- Runs migrations, updates mutators and query definitions
2. **Deploy zero-cache** -- Picks up new schema from PostgreSQL WAL
3. **(If needed) Restart PostgreSQL** -- Only for connection pool issues

## Schema Sync Requirement (3 Locations)

The Zero schema must be synchronized across three locations:

| Location | Purpose | File |
|---|---|---|
| `shared/schema.ts` | Frontend + zero-cache schema definition | PRIMARY source |
| `queries-service/schema.ts` | queries-service mutation validation | Copy of shared |
| PostgreSQL | Actual data storage columns | Managed by migrate.ts |

**Why three copies?** Zero's build system requires the schema at compile time for type safety. The queries-service needs its own copy because it's deployed independently (Railway) and cannot import from the shared directory at runtime. PostgreSQL is the actual database, updated by migrations.

**Sync procedure:** When changing schema, update `shared/schema.ts` first, then `cp shared/schema.ts queries-service/schema.ts`, then add the migration to `migrate.ts`.

## JWT Verification Flow

Authentication uses Clerk JWTs verified against their JWKS endpoint:

```
Request with Authorization: Bearer <JWT>
    |
    v
extractBearerToken() -- Strips "Bearer " prefix
    |
    v
verifyJWT()
    |-- getJWKS() -- Lazily creates remote JWKS set from ZERO_AUTH_JWKS_URL
    |-- jwtVerify(token, keySet, { algorithms: ['RS256', 'ES256'], clockTolerance: 120 })
    |-- Returns payload.sub as userID
```

**Clock tolerance of 120 seconds:** Clerk tokens have a 60-second lifetime. Zero may cache them during sync operations. The 120-second tolerance prevents JWTExpired errors during normal operation.

**Fail-closed design:** If JWKS is not configured, all tokens are rejected (returns `error: 'invalid-token'`). The system never falls back to unsigned verification.

## Key Design Decisions

### Decimal Values as Strings
All price, P&L, and position size fields are stored as `TEXT` in PostgreSQL and `string()` in the Zero schema. This preserves decimal precision -- `f64` would introduce floating-point errors on financial data. The Rust backend uses `rust_decimal::Decimal` for calculations, serializing to strings for storage.

### user_id on Every Table
Every user-scoped table includes `user_id TEXT NOT NULL` even when it could be inferred through relationships (e.g., `strategy_config` has `strategy_id` which has `user_id`). This is intentional -- it enables direct ownership verification without joins, which is critical for the `verifyOwnership()` pattern in mutators.

### Server-Only Tables
`subscription` and `ai_quota` mutators throw unconditionally: `throw new Error('Subscriptions are managed by server webhooks only')`. These tables are written to by Stripe webhooks and server-side quota operations, never by client mutations. The mutator definitions exist to satisfy Zero's type system but reject all client writes.

### Connection Pool Limit
The queries-service uses `postgres(url, { max: 3, idle_timeout: 20 })`. Railway's PostgreSQL has ~20 connections total, shared with zero-cache. Limiting to 3 prevents connection exhaustion.

### No Foreign Keys in PostgreSQL
Tables reference each other by ID (e.g., `strategy_config.strategy_id`) but don't use `REFERENCES` constraints. This is because Zero's sync model doesn't support cascading deletes or referential integrity checks -- the application layer handles consistency.

## Invariants

1. **Schema sync** -- `shared/schema.ts` and `queries-service/schema.ts` MUST be identical. If they diverge, Zero will throw "Column X was not found in the Zero schema" errors.
2. **Query sync** -- `src/queries.ts` and `queries-service/src/queries.ts` MUST define the same queries with the same names and parameter schemas. Missing server-side queries cause "Unknown query: X" errors.
3. **Migration location** -- ALL DDL goes in `queries-service/src/migrate.ts`. CI enforces this.
4. **Push returns 401 on auth failure** -- If the push endpoint returns anything other than 401 for auth failures, Zero's automatic token refresh breaks, causing persistent mutation failures.
5. **user_id forcing on insert** -- Every insert mutator MUST override `user_id` from auth context. Trusting client-supplied user_id is a security vulnerability.
6. **Zero publication** -- New tables that need WAL replication must be added to the `zero_data` publication: `ALTER PUBLICATION zero_data ADD TABLE <name>`.

## Known Technical Debt

1. **Permissions block is ANYONE_CAN** -- The `permissions` export in `shared/schema.ts` uses Zero's legacy RLS system with `ANYONE_CAN` for everything. Security is NOT enforced by permissions -- it's enforced by synced queries (read filtering) and server mutators (write validation). The block exists because Zero requires it but will be removed when Zero drops the legacy system.

2. **Legacy `decodeJWTPayload`** -- `auth.ts` contains a deprecated function that decodes JWTs without verifying signatures. It logs a warning when called. Should be removed once all callers are migrated to `verifyJWT`.

3. **Duplicate query definitions** -- Queries must be defined in both `src/queries.ts` and `queries-service/src/queries.ts` with identical logic. This is a Zero architectural requirement (frontend needs them for optimistic rendering, server needs them for validation), but it creates a maintenance burden.

4. **app.ts is a monolith** -- At ~51KB, `app.ts` contains endpoints for multiple domains. It should ideally be split into route modules, but Hono's routing model and the dependency injection pattern make this non-trivial.

5. **No foreign key constraints** -- Referential integrity is application-enforced. Orphaned records are possible if cleanup logic has bugs.
