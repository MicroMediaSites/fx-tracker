# MCP Server Architecture

> **SUPERSEDED (AGT-649, 2026-07-06):** the MCP server is now a **local stdio
> binary** (`wickd-mcp`) reading the wickd local store (`~/.wickd/app.db`) via
> rusqlite — schema shared with the app through a `#[path]` include of
> `src-tauri/src/local_store/migrations.rs` (see `mcp-server-rs/src/store.rs`).
> Everything below describing HTTP transport, Railway deployment, Clerk
> OAuth/JWT auth, sessions, rate limiting, feature gates, and Postgres is
> retired and deleted from the crate. The `#[tool]`/`#[tool_router]` tool
> surface, shared-type strategy validation, and sanitization notes still apply.

## Overview

The MCP server is a standalone Rust HTTP service (binary: `candlesight-mcp`) deployed on Railway, listening on port 3003. It implements the MCP Streamable HTTP transport -- receiving JSON-RPC requests at `POST /mcp` and returning JSON-RPC responses with `mcp-session-id` headers for session continuity.

It does NOT use rmcp's built-in transport layer. Instead, it manually parses JSON-RPC, dispatches to tools, and formats responses. The rmcp crate is used only for its `#[tool]`/`#[tool_router]`/`#[tool_handler]` macros (tool registration, schema generation, type-safe parameter wrapping).

## Request Flow

```
Claude Desktop / Claude Code
    |
    | POST /mcp  (Bearer token in Authorization header)
    v
axum Router  -->  mcp_handler()
    |
    |-- 1. Extract Bearer token from Authorization header
    |-- 2. Authenticate via TokenAuthenticator (JWT or opaque token)
    |-- 3. Session management (create/validate mcp-session-id)
    |-- 4. Parse JSON-RPC envelope
    |-- 5. Route by method: "initialize", "tools/list", "tools/call", "ping"
    |-- 6. For "tools/call":
    |       a. Rate limit check (per-user, read vs mutation buckets)
    |       b. Feature gate check (subscription tier from DB)
    |       c. Manual tool dispatch via call_tool() match statement
    |       d. Execute tool (DB queries scoped to user_id/account_id)
    |       e. Audit log (structured tracing with timing, resource IDs)
    |       f. Return CallToolResult or sanitized error
    |-- 7. Wrap in JSON-RPC response, attach mcp-session-id header
    v
JSON-RPC Response
```

## Authentication

Two token types are supported, both issued by Clerk:

1. **JWTs** -- Verified locally using RS256 against cached JWKS keys fetched from `{CLERK_FRONTEND_API}/.well-known/jwks.json`. Keys are fetched on demand and cached in memory. Validation uses 30-second clock skew leeway.

2. **Opaque tokens** -- Validated by calling Clerk's `{CLERK_FRONTEND_API}/oauth/userinfo` endpoint with the token. Used as fallback when JWT verification fails (some OAuth flows produce opaque tokens).

The `sub` claim from either path becomes the `user_id` for all downstream queries.

### OAuth Discovery (RFC 9728 / RFC 8414)

Three well-known endpoints are exposed for OAuth discovery:
- `GET /.well-known/oauth-protected-resource` -- Resource metadata pointing to Clerk as authorization server
- `GET /.well-known/oauth-protected-resource/mcp` -- Same metadata scoped to /mcp endpoint (Claude looks here)
- `GET /.well-known/oauth-authorization-server` -- Authorization server metadata (Clerk endpoints)

This allows Claude to automatically discover how to authenticate without manual configuration.

### 401 Responses

Unauthorized responses include `WWW-Authenticate` headers per RFC 6750, with `resource_metadata` pointing to the discovery endpoint. Error types distinguish between no token, invalid token, expired token, and missing subject claim.

## Session Management

Sessions are tracked in-memory (`HashMap<String, Session>`) with a 30-minute timeout. A background task runs every 60 seconds to prune expired sessions. Session IDs are UUIDs assigned on first request and returned via `mcp-session-id` response header.

Sessions are validated for user binding -- a session ID created by user A cannot be used by user B (returns 403).

`DELETE /mcp` terminates a session per the MCP Streamable HTTP spec.

## Authorization Layers

### Rate Limiting (AUDIT-009)

Per-user, in-memory sliding window rate limiter with separate buckets for reads and mutations:
- **Reads**: 100 per 60-second window (list_strategies, get_trades, etc.)
- **Mutations**: 30 per 60-second window (create_strategy, update_note, delete_sr_zone, etc.)

Mutation classification reuses `audit::tool_action()` as the single source of truth -- any tool returning `AuditAction::Create/Update/Delete` is a mutation.

Rate limit exceeded returns HTTP 429 with `Retry-After` header and custom JSON-RPC error code `-32429`.

A background task runs every 120 seconds to clean up stale user buckets.

### Feature Gating (Subscription Tiers)

Tools are gated by subscription tier, checked against the `subscription` table:
- **Free**: Most tools (list_strategies, get_strategy, get_trades, notes CRUD, zones CRUD, pattern matches, backtests, help)
- **Premium**: create_strategy, update_strategy, get_calendar_events
- **Pro**: Reserved for future AI analysis tools

Tier check queries `subscription.tier` and `subscription.status` (only "active" and "past_due" are considered active). On DB error, access is denied (fail-closed).

Feature gate denied returns HTTP 402 with custom JSON-RPC error code `-32402`.

## Data Access Pattern

All database queries are scoped to the authenticated user:
- **Strategies, notes, zones, pattern matches**: Filtered by `user_id = $1`
- **Trades, account summary, open trades**: Filtered by `account_id` resolved from `user_credentials` table based on user's `active_data_source` setting (M65b). Uses the practice or live OANDA account ID accordingly. Old trades without `account_id` are intentionally excluded.

This means the MCP server reads from the same PostgreSQL database as the main application (via queries-service/zero-cache), but all queries go direct to PostgreSQL via sqlx (no zero-cache involvement).

## Strategy Validation

The critical feature of the MCP server is server-side strategy validation. When creating or updating strategies:

1. The strategy JSON is assembled (merging updates with existing fields for update)
2. It is serialized and then deserialized as `shared::StrategyDefinition`
3. If deserialization fails, the strategy is rejected with a descriptive error

This ensures any strategy created via MCP will parse correctly in the backtest engine, since both use the identical Rust types from the `shared` crate.

A `parse_json_value()` helper handles the case where Claude Desktop sends stringified JSON (e.g., indicators as a string rather than array).

## Security Measures

### Output Sanitization (AUDIT-012)

All user-controlled content returned in tool results passes through `sanitize::sanitize_for_tool_result()` which:
- Escapes XML-like tags that could manipulate Claude's context (`</tool_result>`, `<system>`, etc.)
- Strips common prompt injection patterns ("ignore previous instructions", etc.)
- Truncates to 50,000 characters to prevent context stuffing

Applied to: strategy names/descriptions, note titles/content, zone labels, pattern match config names/reasons.

### Input Validation (AUDIT-014)

- All UUID parameters validated with `validate_uuid()` before use in queries
- Price values validated with `validate_price()` (must be valid positive Decimal)
- Price ranges validated with `validate_price_range()` (upper > lower)
- Query limits clamped to `[1, 1000]` via `clamp_limit()`
- Note titles capped at 500 characters, content at 50KB

### Error Sanitization (AUDIT-013)

Internal errors are logged server-side with full details via tracing but return generic messages to the client. Two helper functions enforce this:
- `param_parse_error()` -- logs parse error, returns "Invalid parameters"
- `db_error()` -- logs database error, returns "Operation failed"

Strategy create/update errors intentionally include a hint about common format mistakes (PriceSource vs IndicatorSource syntax) since these are developer-facing tools.

### Deletion Confirmation Gate (AUDIT-009)

`delete_sr_zone` requires `confirm: true` parameter. Without it, the tool returns a warning message describing what will be deleted, requiring the AI to explicitly confirm.

## Database

Connection pool created via `db::create_pool()`:
- Max 5 connections
- 10-second acquire timeout
- TLS enforced (appends `sslmode=require` if not already in DATABASE_URL)

No migrations in this crate -- all schema is managed by `queries-service/src/migrate.ts` per project convention.

## Deployment

- **Platform**: Railway (Dockerfile-based deployment)
- **Port**: 3003 (configurable via PORT env var)
- **Health check**: `GET /health` (returns status, service name, DB connection, active session count)
- **Binary**: `candlesight-mcp`
- **Docker**: Multi-stage build (rust:1.88-slim-bookworm builder, debian:bookworm-slim runtime)
- Help docs are copied into the Docker image (used by `include_str!()` at compile time AND bundled for potential runtime use)

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `DATABASE_URL` | Yes | PostgreSQL connection string |
| `CLERK_FRONTEND_API` | No | Clerk frontend API URL (has default for dev) |
| `MCP_SERVER_URL` | No | Public URL of this server (defaults to localhost:3003) |
| `PORT` | No | Listen port (defaults to 3003) |
| `RUST_LOG` | No | Log filter (defaults to "info") |

## Known Technical Debt

- All tool implementations live in `main.rs` rather than being modularized into `tools/` submodules. The `tools/strategies.rs` file is a placeholder noting this.
- Session storage is in-memory only -- server restart loses all sessions. Not a problem since MCP clients re-authenticate on each connection.
- The `get_calendar_events` tool has a TODO for tier access checking (the feature gate in the dispatch handles it, but there is a redundant comment in the tool body).
- Trade queries use `account_id` lookup which requires an extra DB query per tool call (`get_active_account_id`). Could be cached per session.
- Help docs are both `include_str!()` at compile time AND copied into the Docker image -- the runtime copy is unused.
