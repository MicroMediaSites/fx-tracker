# Data Infrastructure Interfaces

## Synced Queries

All queries use `syncedQueryWithContext` and accept `userID: string | undefined` as the first parameter (from JWT `sub` claim). They are defined in both `src/queries.ts` and `queries-service/src/queries.ts`.

### User
| Query | Parameters | Returns |
|---|---|---|
| `myUser` | (none) | User record for authenticated user |

### Trades
| Query | Parameters | Returns |
|---|---|---|
| `myTrades` | (none) | All trades for user |
| `myTradeById` | `tradeId: string` | Single trade by ID |
| `myOpenTrades` | (none) | Trades where state = 'OPEN' |

### Notes
| Query | Parameters | Returns |
|---|---|---|
| `myNotes` | (none) | All notes for user |
| `myNotesByTrade` | `tradeId: string` | Notes linked to a specific trade |
| `myNotesByStrategy` | `strategyId: string` | Notes linked to a specific strategy |

### Strategies
| Query | Parameters | Returns |
|---|---|---|
| `myStrategies` | (none) | All strategies (including deleted/archived) |
| `myActiveStrategies` | (none) | Active, non-archived strategies (is_active=true, is_archived=false) |
| `myActiveStrategiesWithArchived` | (none) | Active strategies including archived (is_active=true) |
| `myPromotedStrategies` | (none) | Strategies where is_promoted=true |
| `myStrategyById` | `strategyId: string` | Single strategy by ID |

### Backtests
| Query | Parameters | Returns |
|---|---|---|
| `myBacktests` | (none) | All backtests |
| `myBacktestsByStrategy` | `strategyId: string` | Backtests for a strategy |

### Pattern Matches
| Query | Parameters | Returns |
|---|---|---|
| `myPatternMatches` | (none) | All pattern matches |
| `myPatternMatchesByConfig` | `configId: string` | Matches for a strategy config |
| `myPendingPatternMatches` | (none) | Matches where status = 'pending' |

### Strategy Configs
| Query | Parameters | Returns |
|---|---|---|
| `myStrategyConfigs` | (none) | All strategy configurations |
| `myStrategyConfigsByStrategy` | `strategyId: string` | Configs for a strategy |
| `myLiveConfigs` | (none) | Configs where is_live=true |

### Strategy Trades
| Query | Parameters | Returns |
|---|---|---|
| `myStrategyTrades` | (none) | All strategy-trade links |
| `myStrategyTradesByStrategy` | `strategyId: string` | Links for a strategy |
| `myStrategyTradesByTrade` | `tradeId: string` | Links for a trade |

### Strategy Watchers
| Query | Parameters | Returns |
|---|---|---|
| `myStrategyWatchers` | (none) | All watcher configurations |
| `myActiveWatchers` | (none) | Watchers where is_active=true |

### Credentials
| Query | Parameters | Returns |
|---|---|---|
| `myCredentials` | (none) | All user credential records |
| `myCredentialsByDevice` | `deviceId: string` | Credentials for specific device |

### Calendar Events (Global)
| Query | Parameters | Returns |
|---|---|---|
| `upcomingHighImpactEvents` | `fromTimestamp: number` | High-impact events from timestamp onwards |
| `upcomingCalendarEvents` | `fromTimestamp: number` | All events from timestamp onwards |

### S/R Zones
| Query | Parameters | Returns |
|---|---|---|
| `mySRZones` | (none) | All support/resistance zones |
| `mySRZonesByInstrument` | `instrument: string` | Zones for specific instrument |

### Subscriptions & Quota
| Query | Parameters | Returns |
|---|---|---|
| `mySubscription` | (none) | User's subscription record |
| `myAiQuota` | (none) | Current AI quota period (where period_end > now) |
| `myTokenPurchases` | (none) | Completed, non-expired token purchases (FIFO by created_at ASC) |

### Backtest Data
| Query | Parameters | Returns |
|---|---|---|
| `myHoldoutsByInstrument` | `instrument: string` | Holdout quarters for instrument |
| `myContaminationByConfig` | `strategyId, instrument, timeframe` | Contaminated quarters for config |
| `myJobsByStrategy` | `strategyId: string` | All jobs for strategy |
| `myActiveJobsByStrategy` | `strategyId: string` | Pending/running jobs for strategy |
| `myActiveJobs` | (none) | All pending/running jobs |

### Trade Scores
| Query | Parameters | Returns |
|---|---|---|
| `myTradeScores` | (none) | All AI trade scores |
| `myTradeScoreByTrade` | `tradeId: string` | Score for specific trade |

### Chat
| Query | Parameters | Returns |
|---|---|---|
| `myChatMessages` | (none) | All chat messages (ordered by created_at asc) |
| `myPromptHistory` | (none) | Prompt history (ordered by created_at desc) |

### Spread Stats (Global)
| Query | Parameters | Returns |
|---|---|---|
| `spreadStatsByInstrument` | `instrument: string` | Spread statistics for instrument |

---

## Mutators

All mutators require JWT authentication. Server-side mutators enforce ownership and validation. Defined in `queries-service/src/mutators.ts` (server) and `src/mutators.ts` (client optimistic).

### User
| Mutator | Checks | Notes |
|---|---|---|
| `user.insert` | Auth, id === userID | Can only create own user record |
| `user.update` | Auth, id === userID | Can only update own record |
| `user.delete` | Auth, id === userID | Can only delete own record |

### Trade
| Mutator | Checks | Notes |
|---|---|---|
| `trade.insert` | Auth, forces user_id | user_id always set from JWT |
| `trade.update` | Auth, verifyOwnership | Must own the trade |
| `trade.delete` | Auth, verifyOwnership | Must own the trade |

### Note
| Mutator | Checks | Notes |
|---|---|---|
| `note.insert` | Auth, forces user_id | Supports trade_id and strategy_id links |
| `note.update` | Auth, verifyOwnership | |
| `note.delete` | Auth, verifyOwnership | |

### Strategy
| Mutator | Checks | Notes |
|---|---|---|
| `strategy.insert` | Auth, nameUniqueness, validateFields, sanitizeConversation | Full JSON validation |
| `strategy.upsert` | Auth, nameUniqueness (excludes self), validateFields, sanitizeConversation | Used for create-or-update |
| `strategy.update` | Auth, verifyOwnership, nameUniqueness (if name changed), validateFields | Partial updates allowed |
| `strategy.delete` | Auth, verifyOwnership | |

### Backtest
| Mutator | Checks | Notes |
|---|---|---|
| `backtest.insert` | Auth, validateInstrument, validateBacktestFields | Validates results JSON |
| `backtest.delete` | Auth, verifyOwnership | |

### Pattern Match
| Mutator | Checks | Notes |
|---|---|---|
| `pattern_match.insert` | Auth, validateInstrument, validatePatternMatchFields | Validates decimal strings |
| `pattern_match.update` | Auth, verifyOwnership | Status/executed_at only |
| `pattern_match.delete` | Auth, verifyOwnership | |

### Strategy Trade
| Mutator | Checks | Notes |
|---|---|---|
| `strategy_trade.insert` | Auth, forces user_id | Links OANDA trades to strategies |
| `strategy_trade.delete` | Auth, verifyOwnership | |

### Strategy Watcher
| Mutator | Checks | Notes |
|---|---|---|
| `strategy_watcher.insert` | Auth, **requireTier('premium')**, forces user_id | Premium feature gate |
| `strategy_watcher.upsert` | Auth, **requireTier('premium')**, forces user_id | Premium feature gate |
| `strategy_watcher.update` | Auth, verifyOwnership | Toggle active/mode |
| `strategy_watcher.delete` | Auth, verifyOwnership | |

### Strategy Config
| Mutator | Checks | Notes |
|---|---|---|
| `strategy_config.insert` | Auth, validateInstrument, validateTimeframe, validateConfigFields | Full validation |
| `strategy_config.update` | Auth, verifyOwnership, validateConfigFields | Partial updates |
| `strategy_config.delete` | Auth, verifyOwnership | |

### User Credentials
| Mutator | Checks | Notes |
|---|---|---|
| `user_credentials.insert` | Auth, checkAccountIdUniqueness, forces user_id | Prevents duplicate OANDA accounts |
| `user_credentials.upsert` | Auth, checkAccountIdUniqueness (excludes self), forces user_id | |
| `user_credentials.update` | Auth, verifyOwnership, checkAccountIdUniqueness | |
| `user_credentials.delete` | Auth, verifyOwnership | |

### S/R Zone
| Mutator | Checks | Notes |
|---|---|---|
| `sr_zone.insert` | Auth, validateInstrument, forces user_id | |
| `sr_zone.update` | Auth, verifyOwnership | |
| `sr_zone.delete` | Auth, verifyOwnership | |

### Promotion Audit
| Mutator | Checks | Notes |
|---|---|---|
| `promotion_audit.insert` | Auth, forces user_id | Append-only compliance log |

### Subscription, AI Quota & Token Purchases (Server-Only)
| Mutator | Checks | Notes |
|---|---|---|
| `subscription.insert/update/delete` | **Always throws** | Managed by Stripe webhooks only |
| `ai_quota.insert/update/delete` | **Always throws** | Managed by server quota operations only |
| `token_purchase.insert/update/delete` | **Always throws** | Managed by Stripe checkout webhooks only |

### Backtest Holdout / Contamination
| Mutator | Checks | Notes |
|---|---|---|
| `backtest_holdout.insert` | Auth, forces user_id | |
| `backtest_holdout.delete` | Auth, verifyOwnership | |
| `backtest_contamination.insert` | Auth, forces user_id | |
| `backtest_contamination.delete` | Auth, verifyOwnership | |

### Backtest Job
| Mutator | Checks | Notes |
|---|---|---|
| `backtest_job.insert` | Auth, forces user_id | Created by frontend |
| `backtest_job.update` | Auth, verifyOwnership | Status/progress updated by backend via REST |
| `backtest_job.delete` | Auth, verifyOwnership | |

### Chat Message / Prompt History
| Mutator | Checks | Notes |
|---|---|---|
| `chat_message.insert` | Auth, forces user_id | |
| `chat_message.delete` | Auth, verifyOwnership | |
| `prompt_history.insert` | Auth, forces user_id | |
| `prompt_history.delete` | Auth, verifyOwnership | |

### Labels
| Mutator | Checks | Notes |
|---|---|---|
| `label.insert` | Auth, forces user_id | |
| `label.update` | Auth, verifyOwnership | |
| `label.delete` | Auth, verifyOwnership | |
| `trade_label.insert` | Auth, verifyOwnership(trade), verifyOwnership(label), forces user_id | Cross-ownership check |
| `trade_label.delete` | Auth, verifyOwnership | |
| `strategy_label.insert` | Auth, verifyOwnership(strategy), verifyOwnership(label), forces user_id | Cross-ownership check |
| `strategy_label.delete` | Auth, verifyOwnership | |

---

## Schema Tables

26 tables defined in `shared/schema.ts`. All user-scoped tables have `user_id TEXT NOT NULL` indexed.

### Core Entities
| Table | Primary Key | User-scoped | Notes |
|---|---|---|---|
| `user` | `id` (Clerk ID) | id = user | Created via Clerk webhook |
| `trade` | `id` (`userId:oandaId`) | Yes | Synced from OANDA |
| `note` | `id` (UUID) | Yes | Optional trade_id, strategy_id FKs |
| `strategy` | `id` (UUID) | Yes | JSON fields for rules, indicators |
| `backtest` | `id` (UUID) | Yes | JSON results |

### Strategy System
| Table | Primary Key | User-scoped | Notes |
|---|---|---|---|
| `strategy_config` | `id` (UUID) | Yes | Strategy + instrument + timeframe |
| `pattern_match` | `id` (UUID) | Yes | Live detection results |
| `strategy_trade` | `id` (UUID) | Yes | Links OANDA trades to strategies |
| `strategy_watcher` | `id` (config composite) | Yes | Persisted watcher configs |

### Classification & Labels
| Table | Primary Key | User-scoped | Notes |
|---|---|---|---|
| `label` | `id` (UUID) | Yes | User-defined labels |
| `trade_label` | `id` (UUID) | Yes | Junction: trade <-> label |
| `strategy_label` | `id` (UUID) | Yes | Junction: strategy <-> label |

### Authentication & Credentials
| Table | Primary Key | User-scoped | Notes |
|---|---|---|---|
| `user_credentials` | `id` (UUID) | Yes | Encrypted OANDA creds per device |
| `user_api_key` | `id` (UUID) | Yes | One key per user, bcrypt hashed |
| `subscription` | `id` (UUID) | Yes (unique) | Stripe-managed, server-only writes |

### AI Features
| Table | Primary Key | User-scoped | Notes |
|---|---|---|---|
| `ai_quota` | `id` (UUID) | Yes | Token-based usage per period |
| `token_purchase` | `id` (UUID) | Yes | One-time AI token top-ups, server-only writes |
| `trade_score` | `id` (UUID) | Yes | AI trade analysis scores (unique per trade) |
| `chat_message` | `id` (UUID) | Yes | Unified chat history |
| `prompt_history` | `id` (UUID) | Yes | For up-arrow prompt cycling |

### Walk-Forward Testing
| Table | Primary Key | User-scoped | Notes |
|---|---|---|---|
| `backtest_holdout` | `id` (UUID) | Yes | Unique(user, instrument, quarter) |
| `backtest_contamination` | `id` (UUID) | Yes | Unique(user, strategy, instrument, timeframe, quarter) |
| `backtest_job` | `id` (UUID) | Yes | Long-running job tracking |

### Global Data (Not User-Scoped)
| Table | Primary Key | User-scoped | Notes |
|---|---|---|---|
| `calendar_event` | `id` (composite) | No | Economic calendar from Forex Factory |
| `spread_stats` | `instrument` | No | Per-instrument spread EMA/min/max |
| `promotion_audit` | `id` (UUID) | Yes | Compliance logging |
| `waitlist` | `id` (UUID) | No | Beta email collection |

---

## REST Endpoints (This Domain)

### Zero Protocol
| Method | Path | Auth | Description |
|---|---|---|---|
| `POST` | `/get-queries` | JWT | Zero query handler -- zero-cache calls this |
| `POST` | `/push` | JWT (returns 401 on failure) | Zero mutation handler -- zero-cache calls this |

### Health
| Method | Path | Auth | Description |
|---|---|---|---|
| `GET` | `/` | None | Returns `{"status":"ok","service":"candlesight-queries"}` |
| `GET` | `/health` | None | Same as above |

### API Keys
| Method | Path | Auth | Description |
|---|---|---|---|
| `POST` | `/api-key/generate` | JWT | Generate new API key (revokes existing) |
| `POST` | `/api-key/revoke` | JWT | Revoke current API key |
| `GET` | `/api-key/status` | JWT | Check if user has an API key |

### Account Validation
| Method | Path | Auth | Description |
|---|---|---|---|
| `POST` | `/validate-account` | JWT | Pre-check OANDA account ID uniqueness |

### Trade Sync
| Method | Path | Auth | Description |
|---|---|---|---|
| `POST` | `/sync/trades` | JWT | Sync trades from desktop app to PostgreSQL |

### Spread Stats
| Method | Path | Auth | Description |
|---|---|---|---|
| `POST` | `/spread-stats/submit` | None | Submit spread samples from clients |

### Job Tracking
| Method | Path | Auth | Description |
|---|---|---|---|
| `POST` | `/jobs/start` | JWT | Mark job as running |
| `POST` | `/jobs/progress` | JWT | Update job progress (0-100) |
| `POST` | `/jobs/complete` | JWT | Complete job with result JSON |
| `POST` | `/jobs/fail` | JWT | Fail job with error message |
| `POST` | `/jobs/cancel` | JWT | Cancel a running job |

### Releases Proxy
| Method | Path | Auth | Description |
|---|---|---|---|
| `GET` | `/releases/latest.json` | None | Proxy GitHub releases for auto-updater |
| `GET` | `/releases/download/:filename` | None | Proxy release asset download |
| `GET` | `/releases/staging/latest.json` | None | Proxy staging prereleases |
| `GET` | `/releases/staging/download/:filename` | None | Proxy staging asset download |

---

## JWT Verification Interface

Exported from `queries-service/src/auth.ts`:

```typescript
// Full auth extraction (returns userID or undefined)
extractUserID(req: Request): Promise<string | undefined>

// Auth with error detail (for 401 responses)
extractUserIDWithError(req: Request): Promise<AuthResult>
type AuthResult =
  | { userID: string; error?: undefined }
  | { userID?: undefined; error: 'no-token' | 'invalid-token' | 'expired-token' | 'missing-sub' }

// Low-level JWT verification
verifyJWT(token: string): Promise<VerifyResult>
type VerifyResult =
  | { payload: JWTPayload; error?: undefined }
  | { payload?: undefined; error: 'invalid-token' | 'expired-token' }
```

---

## Rate Limiting Interface

Exported from `queries-service/src/rate-limit.ts`:

```typescript
rateLimiter(config?: RateLimitConfig): HonoMiddleware

type RateLimitConfig = {
  windowMs?: number;     // Default: 60000 (1 minute)
  max?: number;          // Default: 100 requests per window
  keyGenerator?: (c: Context) => string;  // Default: IP from x-forwarded-for
  skip?: (c: Context) => boolean;         // Default: skip / and /health
  message?: string;
};
```

Headers set on every response: `X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset`. Returns 429 with `Retry-After` header when exceeded.

---

## How Other Domains Should Interact

### Adding a New Webhook Endpoint (membership-payments, etc.)
1. Create handler function in its own module (e.g., `stripe-webhooks.ts`)
2. Add handler type to `AppDependencies` in `app.ts`
3. Wire the handler in `index.ts` and pass to `createApp()`
4. Add the route in `app.ts` (no auth middleware -- webhooks use their own signature verification)

### Adding a New AI Endpoint (ai-analysis)
1. Add the route in `app.ts` under the AI section
2. Use `extractUserIDWithError()` for auth
3. Check AI quota via `deps.aiQuotaOps.canUseAi()` before processing
4. Increment tokens via `deps.aiQuotaOps.incrementTokenUsage()` after processing

### Reading Data (any domain)
Use synced queries via `useQuery()` in React components. Never query PostgreSQL directly from the frontend.

### Writing Data (any domain)
Use `zero.mutate.<table>.<operation>()` in React components. The mutation flows through Zero to the server mutators automatically.

### Server-Side Data Access (e.g., AI context ops)
Add operations to the `aiContextOps` object in `index.ts` using direct SQL queries against the `sql` connection pool. These bypass Zero's mutation pipeline and are used for read-heavy operations where Zero's optimistic model isn't needed.
