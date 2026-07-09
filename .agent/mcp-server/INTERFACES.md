# MCP Server Interfaces

## MCP Tools Exposed

### Strategy Tools

| Tool | Description | Tier | Mutation |
|------|-------------|------|----------|
| `list_strategies` | List all non-archived strategies for the user | Free | No |
| `get_strategy` | Get a strategy by ID with full definition (indicators, rules, risk settings) | Free | No |
| `create_strategy` | Create a new strategy, validated against shared Rust types | Premium | Yes |
| `update_strategy` | Partial update of an existing strategy, re-validated after merge | Premium | Yes |
| `get_strategy_help` | Return the full strategy-authoring.md guide (compiled into binary) | Free | No |

### Trade Tools

| Tool | Description | Tier | Mutation |
|------|-------------|------|----------|
| `get_account_summary` | Aggregate stats: total trades, wins, losses, win rate, P&L, profit factor | Free | No |
| `get_trades` | Query trade history with filters (instrument, state, date range, limit) | Free | No |
| `get_open_trades` | List currently open positions | Free | No |

### Note Tools

| Tool | Description | Tier | Mutation |
|------|-------------|------|----------|
| `get_notes` | List notes with optional filters (trade_id, strategy_id, search) | Free | No |
| `create_note` | Create a note, optionally linked to a trade or strategy | Free | Yes |
| `update_note` | Update note title and/or content | Free | Yes |

### S/R Zone Tools

| Tool | Description | Tier | Mutation |
|------|-------------|------|----------|
| `get_sr_zones` | List support/resistance zones for an instrument | Free | No |
| `create_sr_zone` | Create a zone with validated price range | Free | Yes |
| `delete_sr_zone` | Delete a zone (requires `confirm: true` safety gate) | Free | Yes |

### Pattern Match Tools

| Tool | Description | Tier | Mutation |
|------|-------------|------|----------|
| `get_pattern_matches` | List recent pattern match signals from strategy watchers | Free | No |
| `update_pattern_match_status` | Mark a signal as "executed" or "dismissed" (only from "pending" state) | Free | Yes |

### Calendar Tools

| Tool | Description | Tier | Mutation |
|------|-------------|------|----------|
| `get_calendar_events` | Query economic calendar with filters (currency, impact, date range) | Premium | No |

### Backtest Tools

| Tool | Description | Tier | Mutation |
|------|-------------|------|----------|
| `get_backtests` | Get backtest results for a strategy (metrics: trades, win rate, PF, return, drawdown, Sharpe) | Free | No |

### Help Tools

| Tool | Description | Tier | Mutation |
|------|-------------|------|----------|
| `get_server_info` | Server version and tool listing summary | Free | No |
| `list_help_topics` | List available help documentation topics | Free | No |
| `get_help` | Get full help document by topic name | Free | No |

## Help Topics

| Topic | File | Description |
|-------|------|-------------|
| `strategy-authoring` | `docs/help/strategy-authoring.md` | Comprehensive V2 strategy schema guide with examples |
| `indicators` | `docs/help/indicators.md` | All 18 indicator types, their parameters, and outputs |

## HTTP Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `GET` | `/` | No | Health check |
| `GET` | `/health` | No | Health check (Railway healthcheck path) |
| `GET` | `/.well-known/oauth-protected-resource` | No | OAuth resource metadata (RFC 9728) |
| `GET` | `/.well-known/oauth-protected-resource/mcp` | No | OAuth resource metadata for /mcp endpoint |
| `GET` | `/.well-known/oauth-authorization-server` | No | OAuth authorization server metadata (RFC 8414) |
| `POST` | `/mcp` | Bearer token | MCP JSON-RPC endpoint |
| `DELETE` | `/mcp` | No | MCP session termination |

## Types Consumed from `shared` Crate

The MCP server imports `StrategyDefinition` from `shared/src/lib.rs` (re-exported via `src/shared/mod.rs`) for strategy validation. This single type transitively pulls in the entire strategy type tree:

### Top-Level
- `StrategyDefinition` -- the root type validated on create/update

### Consumed Transitively (via StrategyDefinition fields)
- `IndicatorConfig`, `IndicatorType`
- `ParameterDefinition`, `ParameterType`, `ParameterizedValue`, `ParameterReference`, `ParameterOption`
- `StrategyVariable`, `VariableExpression`, `VariableSource`, `MathOperator`, `MathOperation`
- `EntryRule` / `AnyEntryRule`, `ExitRule` / `AnyExitRule`
- `Condition`, `TriggerWithNot`, `ChainedTriggerWithNot`, `ChainOperator`
- `TriggerChain`, `ChainedTrigger` (deprecated, kept for backward compat)
- `Trigger` (tagged enum: Givens, Cross, Compare, RiskReward, PercentOfTp, Time, Threshold)
- `GivensTrigger`, `CrossTrigger`, `CompareTrigger`, `RiskRewardTrigger`, `PercentOfTpTrigger`, `TimeTrigger`, `ThresholdTrigger`, `TimeInRangeTrigger`, `DivergenceTrigger`
- `DataSource` (untagged enum: Variable, Indicator, Price, Fixed, Parameter, SRZone, Pivot)
- `IndicatorSource`, `PriceSource`, `FixedSource`, `ParameterSource`, `SRZoneSource`, `PivotSource`
- `RiskSettings`, `RiskMethod`, `StopLossSource`, `StopLossEvaluationMode`
- `EntryLogic`, `EntryLogicMode`
- `RuleDirection`, `CrossDirection`, `ComparisonOperator`, `PriceType`, `CaptureMode`, `TrailConfig`
- `DistanceConfig`, `DistanceUnit`, `DistanceType`
- `MarketRegime`, `DivergenceType`
- `SRTarget`, `SRZone`, `SRZoneDistance`, `SRCondition`
- `PivotLevel`, `PivotPeriod`, `PivotCondition`
- `TimeCondition`, `PositionDirection`

## Database Tables Queried

| Table | Operations | Scoping |
|-------|-----------|---------|
| `strategy` | SELECT, INSERT, UPDATE | `user_id`, filtered by `is_archived = false` |
| `trade` | SELECT | `account_id` (resolved from `user_credentials`) |
| `user_credentials` | SELECT | `user_id` (to resolve active account_id) |
| `note` | SELECT, INSERT, UPDATE | `user_id` |
| `sr_zone` | SELECT, INSERT, DELETE | `user_id` |
| `pattern_match` | SELECT, UPDATE | `user_id` (JOINs `strategy_config` for name/timeframe) |
| `calendar_event` | SELECT | Global (not user-scoped) |
| `backtest` | SELECT | Indirectly via `strategy_id` ownership check |
| `subscription` | SELECT | `user_id` (for feature gate tier check) |

## External APIs Consumed

| API | Purpose | Module |
|-----|---------|--------|
| Clerk JWKS (`/.well-known/jwks.json`) | Fetch RSA public keys for JWT verification | `auth.rs` |
| Clerk Userinfo (`/oauth/userinfo`) | Validate opaque tokens, get user_id | `auth.rs` |

## JSON-RPC Methods Handled

| Method | Description |
|--------|-------------|
| `initialize` | Returns protocol version, capabilities (tools), server info |
| `notifications/initialized` | Client notification, acknowledged with empty response |
| `tools/list` | Returns all tool definitions with schemas |
| `tools/call` | Dispatches to tool implementation (with rate limiting, feature gating, audit) |
| `ping` | Returns empty success response |

## Custom JSON-RPC Error Codes

| Code | Meaning |
|------|---------|
| `-32429` | Rate limit exceeded (includes `Retry-After` header) |
| `-32402` | Payment required (subscription tier insufficient or inactive) |
| `-32700` | Parse error (invalid JSON-RPC) |
| `-32601` | Method not found |
| `-32000` | Tool execution failed (generic, details logged server-side) |

## Tool Parameter Types (`types/params.rs`)

All parameter structs derive `Debug, Serialize, Deserialize, JsonSchema`. Key validation constants:

| Constant | Value | Purpose |
|----------|-------|---------|
| `MAX_LIMIT` | 1000 | Upper bound for all query limits |
| `MAX_CONTENT_LENGTH` | 51200 (50KB) | Note content size cap |
| `MAX_TITLE_LENGTH` | 500 | Note/strategy title cap |

Validation helpers:
- `validate_uuid(id, field_name)` -- returns `Option<String>` error message
- `validate_price(price, field_name)` -- parses to positive Decimal
- `validate_price_range(upper, lower)` -- validates both prices and upper > lower
- `clamp_limit(limit)` -- clamps to `[1, MAX_LIMIT]`
