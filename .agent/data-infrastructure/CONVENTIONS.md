# Data Infrastructure Conventions

## How to Add a New Database Table

Full checklist, in order:

### 1. Add Migration to `queries-service/src/migrate.ts`

```typescript
// Create the table
await sql`
  CREATE TABLE IF NOT EXISTS my_table (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    data TEXT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
  )
`.catch((err) => {
  console.error('[migrations] Error creating my_table:', err.message);
});

// Add indexes for common query patterns
await sql`CREATE INDEX IF NOT EXISTS idx_my_table_user_id ON my_table(user_id)`.catch(() => {});
```

Convention: Use `BIGINT NOT NULL` for timestamps (Unix milliseconds). Use `TEXT` for all string and decimal fields. Use `BOOLEAN` for flags. Always include a `user_id TEXT NOT NULL` column for user-scoped tables.

If the table needs WAL replication for Zero sync (most do), add it to the publication:
```typescript
await sql`ALTER PUBLICATION zero_data ADD TABLE my_table`.catch(() => {});
```

### 2. Add to Zero Schema in `shared/schema.ts`

```typescript
const myTable = table('my_table')
  .columns({
    id: string(),
    user_id: string(),
    name: string(),
    data: string().optional(),  // nullable columns use .optional()
    created_at: number(),
    updated_at: number(),
  })
  .primaryKey('id');
```

Then register in the `createSchema` call:
```typescript
export const schema = createSchema({
  tables: [
    // ... existing tables
    myTable,
  ],
});
```

### 3. Copy Schema to Queries-Service

```bash
cp shared/schema.ts queries-service/schema.ts
```

These two files MUST be identical. The queries-service has its own copy because it deploys independently to Railway.

### 4. Add Queries to Both Files

Add to `queries-service/src/queries.ts`:
```typescript
export const myItems = syncedQueryWithContext(
  'myItems',
  z.tuple([]),
  (userID: string | undefined) => {
    if (!userID) return builder.my_table.where('id', '___never_match___');
    return builder.my_table.where('user_id', userID);
  }
);
```

Add the IDENTICAL definition to `src/queries.ts` (only the import path for `builder` differs).

### 5. Register Query in `queries-service/src/index.ts`

Import the query and add to `validatedQueries`:
```typescript
import { myItems } from './queries.js';

const validatedQueries = {
  // ... existing
  [myItems.queryName]: withValidation(myItems),
};
```

### 6. Add Mutators to Both Files

Add to `queries-service/src/mutators.ts` (with full security):
```typescript
my_table: {
  insert: async (tx: Transaction<Schema>, args: {
    id: string;
    user_id: string;
    name: string;
    data?: string | null;
    created_at: number;
    updated_at: number;
  }) => {
    const userID = requireAuth();
    // Force user_id from auth - never trust client
    await tx.mutate.my_table.insert({ ...args, user_id: userID });
  },
  update: async (tx: Transaction<Schema>, args: {
    id: string;
    name?: string;
    data?: string | null;
    updated_at?: number;
  }) => {
    const userID = requireAuth();
    await verifyOwnership(tx, 'my_table', args.id, userID);
    await tx.mutate.my_table.update({ ...args, updated_at: Date.now() });
  },
  delete: async (tx: Transaction<Schema>, args: { id: string }) => {
    const userID = requireAuth();
    await verifyOwnership(tx, 'my_table', args.id, userID);
    await tx.mutate.my_table.delete(args);
  },
},
```

Add to `src/mutators.ts` (optimistic, no security):
```typescript
my_table: {
  insert: async (tx: Transaction<Schema>, args: { /* same shape */ }) => {
    await tx.mutate.my_table.insert(args);
  },
  update: async (tx: Transaction<Schema>, args: { /* same shape */ }) => {
    await tx.mutate.my_table.update({ ...args, updated_at: Date.now() });
  },
  delete: async (tx: Transaction<Schema>, args: { id: string }) => {
    await tx.mutate.my_table.delete(args);
  },
},
```

### 7. Rebuild and Deploy

```bash
npm run predev   # Rebuilds schema.cjs from shared/schema.ts
```

Deploy queries-service first (runs migrations), then zero-cache.

---

## How to Add a New Column to an Existing Table

1. **Add ALTER TABLE to `migrate.ts`:**
   ```typescript
   await sql`ALTER TABLE my_table ADD COLUMN IF NOT EXISTS new_column TEXT`.catch(() => {});
   ```

2. **Add to Zero schema** in `shared/schema.ts`:
   ```typescript
   new_column: string().optional(),  // Must be optional for backward compat
   ```

3. **Copy schema:** `cp shared/schema.ts queries-service/schema.ts`

4. **Update mutators** if the column should be settable by clients (add to args type in both files).

5. **Update queries** if the column needs to be filterable.

New columns on existing tables MUST be nullable (use `.optional()` in Zero, no `NOT NULL` in migration) for backward compatibility with existing rows.

---

## How to Add a New Query

1. **Define in `queries-service/src/queries.ts`** using `syncedQueryWithContext`
2. **Define identical query in `src/queries.ts`**
3. **Import and register** in `queries-service/src/index.ts`:
   ```typescript
   import { myNewQuery } from './queries.js';
   const validatedQueries = {
     [myNewQuery.queryName]: withValidation(myNewQuery),
   };
   ```

If you forget step 3, you get: `Unknown query: myNewQuery`

---

## How to Add a New Mutator

1. **Define in `queries-service/src/mutators.ts`** with full security (requireAuth, verifyOwnership, validation)
2. **Define in `src/mutators.ts`** with optimistic-only logic (no auth checks)
3. **Ensure the Mutators type** in `src/mutators.ts` is updated if adding a new entity

If you forget the server-side definition, you get: `could not find mutator my_table|insert`

---

## Naming Conventions

### Query Names
- **`my*`** prefix for user-scoped queries: `myTrades`, `myStrategies`, `myNotes`
- **`my*By<Field>`** for filtered variants: `myTradeById`, `myNotesByTrade`, `myBacktestsByStrategy`
- **`my<Adjective><Entity>`** for state-filtered: `myActiveStrategies`, `myPendingPatternMatches`, `myLiveConfigs`
- **No prefix** for global data: `upcomingHighImpactEvents`, `spreadStatsByInstrument`

### Table Names
- snake_case: `strategy_config`, `pattern_match`, `backtest_holdout`
- Junction tables use both entity names: `trade_label`, `strategy_label`

### Column Names
- snake_case: `user_id`, `created_at`, `is_active`
- Foreign keys: `<entity>_id` (e.g., `strategy_id`, `trade_id`)
- Timestamps: `created_at`, `updated_at`, `completed_at`, `executed_at` (always BIGINT, Unix ms)
- Boolean flags: `is_*` prefix: `is_active`, `is_promoted`, `is_locked`, `is_archived`
- JSON fields: stored as `TEXT` with descriptive names: `indicators`, `entry_rules`, `risk_settings`

---

## Validation Patterns

### Zod Schemas in `validation.ts`

JSON fields stored as TEXT are validated before insertion:

```typescript
// Strategy JSON fields are parsed and validated against Zod schemas
const validationError = validateStrategyFields({
  indicators: args.indicators,      // Must be JSON array of IndicatorConfigSchema
  entry_rules: args.entry_rules,    // Must be JSON array of EntryRuleSchema
  entry_logic: args.entry_logic,    // Must be JSON object of EntryLogicSchema
  exit_rules: args.exit_rules,      // Must be JSON array of ExitRuleSchema
  risk_settings: args.risk_settings, // Must be JSON object of RiskSettingsSchema
});
if (validationError) {
  throw new Error(`Strategy validation failed: ${validationError}`);
}
```

Available validators:
- `validateStrategyFields()` -- Validates strategy JSON columns
- `validateStrategyConfigFields()` -- Validates indicator_params and risk_overrides
- `validatePatternMatchFields()` -- Validates decimal string fields (prices, sizes)
- `validateBacktestFields()` -- Validates results JSON
- `validateInstrument()` -- Validates `XXX_XXX` format
- `validateTimeframe()` -- Validates against whitelist: `M1, M5, M15, M30, H1, H4, D, W, M`
- `validatePlanningConversation()` -- Validates + sanitizes AI planning conversation (AUDIT-008)

### Decimal String Validation

```typescript
const decimalString = z.string().refine(
  (val) => { const num = parseFloat(val); return !isNaN(num) && isFinite(num); },
  { message: 'Must be a valid decimal number string' }
);
```

### Instrument Format
Must match `/^[A-Z]{3}_[A-Z]{3}$/` (e.g., `EUR_USD`, `GBP_JPY`).

---

## Error Handling Patterns

### Mutator Errors
Mutators throw plain `Error` objects with descriptive messages. Zero catches these and rolls back the optimistic update on the client:

```typescript
throw new Error('Authentication required');
throw new Error('Access denied to strategy: <id>');
throw new Error('Strategy validation failed: indicators must be a JSON array');
throw new Error('ACCOUNT_NOT_AVAILABLE');  // Generic to prevent enumeration
```

### Endpoint Errors
REST endpoints return JSON with `error` field and appropriate HTTP status:

```typescript
return c.json({ error: 'Authentication required' }, 401);
return c.json({ error: 'An error occurred processing your request' }, 500);
```

Internal error details are logged but NOT returned to the client (security: prevents information leakage).

### Migration Errors
Each migration wraps in `.catch()` and logs but continues. This allows the server to start even if some migrations fail (they will succeed on next deploy):

```typescript
await sql`ALTER TABLE X ADD COLUMN IF NOT EXISTS Y TEXT`.catch(() => {});
```

---

## The `___never_match___` Pattern

When `userID` is undefined, synced queries must still return a valid Zero query builder object. The convention is to filter on `where('id', '___never_match___')`, which produces a query that matches zero rows:

```typescript
export const myTrades = syncedQueryWithContext(
  'myTrades',
  z.tuple([]),
  (userID: string | undefined) => {
    if (!userID) return builder.trade.where('id', '___never_match___');
    return builder.trade.where('user_id', userID);
  }
);
```

Why not just `where('user_id', '')`? Because an empty string is a valid value that could theoretically match. `___never_match___` is guaranteed to not be a valid ID.

---

## Anti-Patterns

### NEVER: Migrations in Rust
```
// WRONG - src-tauri/src/db.rs
sqlx::query("CREATE TABLE IF NOT EXISTS ...").execute(&pool).await?;
```
All DDL goes in `queries-service/src/migrate.ts`. The Rust backend only runs SELECT/INSERT/UPDATE/DELETE queries.

### NEVER: Schema Divergence
```
// WRONG - different columns in different files
// shared/schema.ts has: account_id: string().optional()
// queries-service/schema.ts does NOT have account_id
```
These two files must be byte-for-byte identical (except the file path itself).

### NEVER: Trusting Client user_id
```typescript
// WRONG
await tx.mutate.note.insert(args);  // Client could send any user_id

// RIGHT
await tx.mutate.note.insert({ ...args, user_id: userID });  // Force from JWT
```

### NEVER: Skipping verifyOwnership on Update/Delete
```typescript
// WRONG - user could modify another user's records
await tx.mutate.strategy.update(args);

// RIGHT
await verifyOwnership(tx, 'strategy', args.id, userID);
await tx.mutate.strategy.update(args);
```

### NEVER: Context as an Object
```typescript
// WRONG - context is string | undefined, NOT an object
syncedQueryWithContext('q', z.tuple([]), (ctx: { userID: string }) => ...)

// RIGHT
syncedQueryWithContext('q', z.tuple([]), (userID: string | undefined) => ...)
```

### NEVER: Returning Internal Error Details
```typescript
// WRONG
return c.json({ error: error.message, stack: error.stack }, 500);

// RIGHT
console.error('[queries-service] Push error:', error);
return c.json({ error: 'An error occurred processing your request' }, 500);
```

### NEVER: Adding NOT NULL Columns Without Defaults
New columns on existing tables must be nullable or have a DEFAULT value. Zero doesn't support NOT NULL columns that don't have defaults, because existing rows would violate the constraint.
