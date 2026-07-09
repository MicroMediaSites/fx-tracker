# MCP Server Domain

HTTP-based MCP (Model Context Protocol) server that exposes CandleSight trading data and operations to external AI tools (Claude Desktop, Claude Code, etc.). Provides strategy CRUD, trade queries, notes, S/R zones, pattern matches, calendar events, and backtest results -- all scoped to the authenticated user.

## Owned Files

```
mcp-server-rs/src/main.rs
mcp-server-rs/src/auth.rs
mcp-server-rs/src/db.rs
mcp-server-rs/src/audit.rs
mcp-server-rs/src/rate_limit.rs
mcp-server-rs/src/sanitize.rs
mcp-server-rs/src/feature_gate.rs
mcp-server-rs/src/types/mod.rs
mcp-server-rs/src/types/params.rs
mcp-server-rs/src/types/db_rows.rs
mcp-server-rs/src/tools/mod.rs
mcp-server-rs/src/tools/strategies.rs
mcp-server-rs/src/shared/mod.rs
mcp-server-rs/docs/help/strategy-authoring.md
mcp-server-rs/docs/help/indicators.md
mcp-server-rs/Cargo.toml
mcp-server-rs/Cargo.lock
mcp-server-rs/Dockerfile
mcp-server-rs/railway.toml
mcp-server-rs/.cargo/config.toml
```

## Shared Files (Coordinate with Other Domains)

| File | Other Domain | Coordination Notes |
|------|-------------|-------------------|
| `shared/src/lib.rs` | `backtest-core`, `strategy-monitor`, `indicators` | All strategy type definitions (`StrategyDefinition`, `Trigger`, `DataSource`, `RiskSettings`, etc.). The MCP server imports `StrategyDefinition` (via `src/shared/mod.rs` re-export) to validate strategies on create/update -- ensuring they parse identically to the backtest engine. Changes to shared types require updating MCP server validation. The `.cargo/config.toml` patches the git dependency to use the local `../shared` path for development. |

## Primary Languages and Frameworks

- **Rust** (all code)
- **axum 0.8** (HTTP server, routing, middleware)
- **rmcp 0.12** (MCP SDK -- `#[tool]`, `#[tool_router]`, `#[tool_handler]` macros for tool registration and schema generation)
- **sqlx 0.8** (async PostgreSQL queries)
- **schemars 1** (JSON Schema generation for MCP tool parameter introspection)

## Key Dependencies

### External Crates
- `rmcp` -- MCP protocol types (`ServerHandler`, `CallToolResult`, `Content`, `ErrorData`, tool macros)
- `axum` / `tower` / `tower-http` -- HTTP framework with CORS support
- `sqlx` -- async PostgreSQL connection pool and typed queries
- `schemars` -- derives `JsonSchema` on all tool param structs so the MCP SDK can expose input schemas
- `jsonwebtoken` -- JWT decoding and RS256 verification against Clerk JWKS
- `reqwest` -- HTTP client for fetching JWKS keys and Clerk userinfo (opaque token validation)
- `rust_decimal` -- decimal arithmetic for price validation (`validate_price`, `validate_price_range`)
- `serde` / `serde_json` -- serialization throughout
- `uuid` -- generating resource IDs for new strategies, notes, zones
- `chrono` -- timestamp formatting and date parsing
- `tracing` / `tracing-subscriber` -- structured logging with env-filter
- `thiserror` -- error type derivation (DbError)
- `dotenvy` -- `.env` loading

### Internal Modules
- `shared` crate (`StrategyDefinition`) -- consumed for strategy validation on create/update
