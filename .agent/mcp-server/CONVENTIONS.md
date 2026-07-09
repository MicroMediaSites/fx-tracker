# MCP Server Conventions

## Adding a New MCP Tool

Adding a tool requires changes in four places within `main.rs` plus a parameter struct in `types/params.rs`:

### 1. Define parameter struct (`types/params.rs`)

```rust
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MyNewToolParams {
    /// Required field (appears in MCP schema description)
    pub some_id: String,
    /// Optional field with default
    #[serde(default)]
    pub limit: Option<i32>,
}
```

- Derive `JsonSchema` (from `schemars`) so rmcp can expose the input schema to AI clients.
- Use `#[serde(default)]` for optional fields. Use `#[serde(default = "default_limit_20")]` for custom defaults.
- Add doc comments on every field -- these become the `description` in the MCP tool schema that the AI reads.

### 2. Add the tool method on `CandlesightMcp` (inside `#[tool_router] impl`)

```rust
#[tool(description = "Human-readable description of what this tool does")]
async fn my_new_tool(&self, params: Parameters<MyNewToolParams>) -> Result<CallToolResult, McpError> {
    let p = params.0;
    // ... implementation
    Ok(CallToolResult::success(vec![Content::text(result_string)]))
}
```

- The `#[tool(description = "...")]` attribute generates the tool listing entry.
- Always accept `Parameters<T>` wrapper and destructure with `params.0`.
- Return `CallToolResult::success(vec![Content::text(...)])` for success.
- Return `Err(McpError::invalid_params(...))` for user input errors.
- Use `db_error()` or `param_parse_error()` helpers for internal/parse errors -- never expose raw error messages.

### 3. Add dispatch entry in `call_tool()` match

```rust
"my_new_tool" => {
    let params: MyNewToolParams = serde_json::from_value(arguments)
        .map_err(|e| param_parse_error("my_new_tool", &self.user_id, e))?;
    self.my_new_tool(Parameters(params)).await
}
```

This manual dispatch is required because the server does not use rmcp's built-in transport layer.

### 4. Update audit and rate limit metadata

In `audit.rs`:
- Add the tool name to `tool_action()` if it is a mutation (CREATE/UPDATE/DELETE).
- Add to `resource_type()` to classify the resource.
- Add to `extract_resource_id()` to log relevant IDs.

In `feature_gate.rs`:
- Add to `required_tier()` if the tool requires Premium or Pro subscription.

### 5. Add DB row type if needed (`types/db_rows.rs`)

```rust
#[derive(sqlx::FromRow)]
pub struct MyNewRow {
    pub id: String,
    pub name: String,
    // ...
}
```

## Naming Patterns

### Tool Names
- Read operations: `get_*` (single) or `list_*` (collection)
- Create operations: `create_*`
- Update operations: `update_*`
- Delete operations: `delete_*`
- Info/help: `get_server_info`, `get_help`, `list_help_topics`, `get_strategy_help`

### Parameter Struct Names
- Follow the pattern `{ToolName}Params` in PascalCase: `GetTradesParams`, `CreateNoteParams`, `DeleteZoneParams`

### DB Row Struct Names
- Follow the pattern `{Entity}Row` or `Full{Entity}Row`: `StrategyRow`, `FullStrategyRow`, `TradeRow`, `FullTradeRow`

## Error Handling

### Never expose internal errors to the client

Every database error or internal failure must:
1. Log the full error server-side via `tracing::error!()` with `user_id` and operation context
2. Return a generic message to the client

Use the helper functions:
```rust
// For parameter parsing failures
param_parse_error("tool_name", &self.user_id, serde_error)

// For database operation failures
db_error("operation_name", &self.user_id, sqlx_error)
```

For inline errors where helpers are not a fit:
```rust
.map_err(|e| {
    tracing::error!(user_id = %self.user_id, error = %e, "describe what failed");
    McpError::internal_error("Generic message for client", None)
})?;
```

### User-facing error messages

- Parameter errors: Use `McpError::invalid_params(message, None)` with a helpful message
- Not found: `McpError::invalid_params("Resource not found", None)` (not 404, since this is JSON-RPC)
- Ownership violations: `McpError::invalid_params("Resource not found or not owned by you", None)`
- Strategy validation: Include hints about common format mistakes (PriceSource vs IndicatorSource)

## Security Checklist for Every Tool

1. **Validate UUID inputs** with `validate_uuid()` before using in queries
2. **Scope all queries** to `user_id = $1` (or `account_id` for trades)
3. **Sanitize user-controlled output** with `sanitize::sanitize_for_tool_result()` before returning strings that originated from user input (names, descriptions, content, labels)
4. **Validate content bounds** -- enforce `MAX_TITLE_LENGTH` (500) and `MAX_CONTENT_LENGTH` (50KB) for text inputs
5. **Clamp limits** with `clamp_limit()` to prevent unbounded queries
6. **Verify ownership** before mutations -- check that the resource belongs to the user before updating or deleting
7. **Require confirmation** for destructive operations (see `delete_sr_zone` pattern)

## Output Formatting

- Return pretty-printed JSON for structured data: `serde_json::to_string_pretty(&value).unwrap()`
- Return plain text for simple messages: `format!("Created note \"{}\" with ID: {}", title, id)`
- Format timestamps as ISO 8601: use `format_timestamp()` helper
- Format monetary values with `$` prefix and 2 decimal places: `format!("${:.2}", amount)`
- Format percentages with 1 decimal: `format!("{:.1}%", rate)`
- Trade direction: derive from units sign (`if units > 0.0 { "LONG" } else { "SHORT" }`)

## Database Query Patterns

### Always scope to user
```rust
"SELECT ... FROM table WHERE user_id = $1"
```

### For trade-related queries, use account_id
```rust
let account_id = self.get_active_account_id().await;
"SELECT ... FROM trade WHERE account_id = $1"
```

### Use nullable parameter binding for optional filters
```rust
"AND ($2::text IS NULL OR instrument = $2)"
.bind(&p.instrument)  // Option<String> -- binds as NULL when None
```

### Always use clamp_limit for LIMIT clauses
```rust
"LIMIT $3"
.bind(clamp_limit(p.limit))
```

## Anti-Patterns

### Do not add migrations to this crate
All database schema changes must go in `queries-service/src/migrate.ts`. The MCP server only reads/writes data, never alters schema.

### Do not bypass the manual dispatch
Even though rmcp's `#[tool_router]` macro generates routing, the server uses manual dispatch in `call_tool()`. Every new tool must be added to the match statement. This is because the server handles its own HTTP transport rather than using rmcp's.

### Do not return raw sqlx errors or serde errors to the client
Always use `param_parse_error()`, `db_error()`, or a custom `map_err` that logs internally and returns a generic message.

### Do not skip output sanitization for user-controlled strings
Any field that a user can set (strategy name, note content, zone label, etc.) must pass through `sanitize_for_tool_result()` before being included in a tool result. This prevents prompt injection attacks.

### Do not use f64 for price validation
Price validation uses `rust_decimal::Decimal` via `validate_price()`. Strategy validation uses the shared crate types which also use Decimal internally.

### Do not hardcode tier checks inline
All tier requirements should go in `feature_gate::required_tier()`. The dispatcher calls `check_feature_access()` generically for every tool.

### Do not add help docs outside the docs/help/ directory
Help content is loaded via `include_str!()` at compile time. New topics need a markdown file in `docs/help/` and entries in both `list_help_topics()` and the `get_help()` match statement.
