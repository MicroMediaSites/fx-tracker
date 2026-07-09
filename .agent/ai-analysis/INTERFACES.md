# AI Analysis Interfaces

## Tauri Commands

### Chat Commands (`src-tauri/src/commands/chat.rs`)

#### `chat_stream`

Starts a streaming chat session with tool support. Spawns a background task.

```rust
#[tauri::command]
pub async fn chat_stream(
    session_id: String,
    context: ChatContext,         // Window context (tagged enum)
    message: String,              // User's message (will be sanitized)
    history: Vec<ChatMessage>,    // Previous messages
    model: Option<String>,        // "haiku" | "sonnet" | "opus"
    enable_tools: Option<bool>,   // Default: true
    user_id: Option<String>,
    user_tier: Option<String>,    // "free" | "premium" | "pro"
    auth_token: Option<String>,   // JWT from Clerk
    state: State<'_, AppState>,
    chat_state: State<'_, ChatSessionState>,
    app_handle: AppHandle,
) -> Result<(), String>
```

**Requires**: `QUERIES_SERVICE_URL` configured, `auth_token` provided.
**Rate limited**: 500ms minimum between requests.

#### `chat_cancel`

Cancels an active streaming session.

```rust
#[tauri::command]
pub async fn chat_cancel(
    session_id: String,
    chat_state: State<'_, ChatSessionState>,
) -> Result<(), String>
```

#### `is_chat_enabled`

Returns true if `QUERIES_SERVICE_URL` is configured.

```rust
#[tauri::command]
pub async fn is_chat_enabled(state: State<'_, AppState>) -> Result<bool, String>
```

#### `create_chat_compaction`

Compacts chat history by merging oldest message with existing compaction via Haiku.

```rust
#[tauri::command]
pub async fn create_chat_compaction(
    input: CompactionInput,       // { oldest_message, existing_compaction }
    auth_token: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String>       // Returns compaction summary text
```

### Analysis Commands (`src-tauri/src/commands/analysis.rs`)

#### `enrich_trades`

Adds time context (hour, day, session, duration, direction) to raw trades.

```rust
#[tauri::command]
pub async fn enrich_trades(
    trades: Vec<enrichment::TradeInput>,
) -> Result<Vec<enrichment::EnrichedTrade>, String>
```

#### `analyze_trade_patterns`

Returns statistical breakdowns by dimension.

```rust
#[tauri::command]
pub async fn analyze_trade_patterns(
    trades: Vec<enrichment::EnrichedTrade>,
) -> Result<statistics::AnalysisResult, String>
```

#### `analyze_advanced_patterns`

Detects streaks, correlations, and time edges.

```rust
#[tauri::command]
pub async fn analyze_advanced_patterns(
    trades: Vec<enrichment::EnrichedTrade>,
) -> Result<patterns::PatternAnalysis, String>
```

#### `analyze_trade_review`

Deep-dive analysis of a single trade: entry timing, exit quality, post-exit movement.

```rust
#[tauri::command]
pub async fn analyze_trade_review(
    trade_id: String,
    instrument: String,
    units: String,
    open_price: String,
    close_price: String,
    open_time: i64,           // timestamp in milliseconds
    close_time: i64,
    realized_pl: String,
    granularity: String,
    state: State<'_, AppState>,
) -> Result<trade_review::TradeReview, String>
```

Fetches 50 candles before entry, all candles during trade, and 20 candles after exit from OANDA.

#### `get_trade_indicator_context`

Computes indicator values at entry and exit for AI analysis.

```rust
#[tauri::command]
pub async fn get_trade_indicator_context(
    instrument: String,
    open_time: i64,
    close_time: i64,
    units: String,
    granularity: String,
    open_price: Option<String>,
    close_price: Option<String>,
    realized_pl: Option<String>,
    state: State<'_, AppState>,
) -> Result<trade_review::TradeAnalysisContext, String>
```

## Queries-Service Endpoints

### `/ai/chat` (POST)

Streaming or non-streaming chat through the Anthropic proxy.

**Request body** (from `ProxyChatRequest`):
```json
{
  "model": "opus",
  "maxTokens": 8192,
  "context": { "type": "backtesting", ... },
  "userTier": "pro",
  "messages": [{ "role": "user", "content": "..." }],
  "tools": [{ "name": "...", "description": "...", "input_schema": {...} }],
  "stream": true
}
```

**Response**: SSE stream (Content-Type: text/event-stream) passthrough from Anthropic.

When `stream: false`, returns:
```json
{
  "content": [{ "type": "text", "text": "..." }],
  "model": "claude-opus-4-5-20251101",
  "usage": { "input_tokens": 1234, "output_tokens": 567 }
}
```

**Error 403** (quota exceeded):
```json
{
  "error": "AI quota exceeded",
  "reason": "limit_reached" | "trial_expired" | "no_quota",
  "remaining": 0,
  "limit": 500000
}
```

### `/ai/trade-analysis` (POST)

Non-streaming trade analysis.

**Request body**:
```json
{
  "model": "opus",
  "prompt": "Full analysis prompt including trade context..."
}
```

**Response**:
```json
{
  "text": "AI analysis response...",
  "inputTokens": 1234,
  "outputTokens": 567
}
```

### `/ai/compact` (POST)

Chat history compaction using Haiku.

**Request body**:
```json
{
  "content": "Message content to compact...",
  "existingCompaction": "Previous compaction summary or null"
}
```

**Response**:
```json
{
  "text": "Compacted summary...",
  "inputTokens": 200,
  "outputTokens": 100
}
```

### `/ai/classify` (POST)

Prompt classification using GPT-4o-mini.

**Request body**:
```json
{
  "prompt": "User's message",
  "windowType": "backtesting",
  "windowContext": "Strategy: EMA Crossover...",
  "recentHistory": [{ "role": "user", "content": "..." }]
}
```

**Response**:
```json
{
  "primary": "backtest",
  "secondary": null,
  "confidence": "high",
  "reasoning": "User is asking about backtest metrics",
  "source": "gpt-4o-mini"
}
```

Categories: `trade`, `strategy`, `backtest`, `market`, `general`, `app_help`

### `/ai/quota-status` (GET)

Check user's AI token quota.

**Response**:
```json
{
  "allowed": true,
  "remaining": 450000,
  "limit": 500000,
  "model": "opus",
  "is_trial": false,
  "trial_days_left": null
}
```

### `/ai/increment-tokens` (POST)

Record token usage.

**Request body**:
```json
{
  "inputTokens": 1234,
  "outputTokens": 567
}
```

**Response**:
```json
{
  "success": true,
  "totalUsed": 50000,
  "remaining": 450000
}
```

### `/ai/create-trial` (POST)

Create trial quota for free-tier user.

**Response**:
```json
{
  "success": true,
  "quota": {
    "model": "haiku",
    "limit": 100000,
    "trialExpiresAt": 1708000000000
  }
}
```

## AI Tool Interface

### Tool Definition Format

Matches Anthropic's tool use API:

```rust
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,  // JSON Schema object
}
```

### Tool Execution

```rust
pub async fn execute_tool(
    tool_use_id: &str,
    name: &str,
    input: serde_json::Value,
    context: &ToolContext,
) -> ToolResult

pub struct ToolResult {
    pub tool_use_id: String,
    pub content: String,     // JSON string (always)
    pub is_error: bool,
}
```

### Tool Context

```rust
pub struct ToolContext {
    pub oanda_client: Arc<RwLock<OandaClient>>,
    pub queries_service: Option<Arc<QueriesServiceClient>>,
    pub auth_token: Option<String>,
    pub user: Option<UserContext>,
    pub app_handle: AppHandle,
}

pub struct UserContext {
    pub user_id: String,
    pub tier: String,  // "free", "premium", "pro"
}
```

### Tool List (26 tools)

| Name | Category | Auth Required | Tier Gate |
|------|----------|--------------|-----------|
| `get_current_time` | Utility | No | None |
| `get_current_price` | Market | No | None |
| `get_recent_candles` | Market | No | None |
| `get_account_summary` | Account | No | None |
| `get_open_positions` | Account | No | None |
| `get_pending_orders` | Account | No | None |
| `get_trade_history` | Account | No | None |
| `get_strategies` | Strategy | Yes | None |
| `get_strategy_details` | Strategy | Yes | None |
| `create_strategy` | Strategy | Yes | Pro |
| `update_strategy` | Strategy | Yes | Pro |
| `promote_strategy` | Strategy | Yes | None |
| `get_backtest_results` | Backtest | Yes | None |
| `list_backtest_jobs` | Backtest | Yes | None |
| `check_strategy_feasibility` | Feasibility | No | None |
| `get_notes` | Notes | Yes | None |
| `get_trade_notes` | Notes | Yes | None |
| `get_strategy_notes` | Notes | Yes | None |
| `get_trade_analytics` | Analytics | Yes | None |
| `get_trades_list` | Analytics | Yes | None |
| `get_economic_calendar` | Calendar | Yes | Premium/Pro |
| `open_windows` | Window Mgmt | No | None |
| `update_ticket` | Ticket | No | None |
| `start_monitor` | Monitor | No | None |
| `stop_monitor` | Monitor | No | None |
| `add_monitor_instruments` | Monitor | No | None |
| `remove_monitor_instruments` | Monitor | No | None |

## Events Emitted

### Chat Streaming Events

| Event | Payload | Emitted By |
|-------|---------|-----------|
| `chat-token` | `{ session_id, text }` | `streaming.rs`, `proxy_client.rs` during streaming |
| `chat-complete` | `{ session_id, full_text, input_tokens, output_tokens, stop_reason }` | `streaming.rs`, `proxy_client.rs` on stream completion |
| `chat-error` | `{ session_id, error_type, message }` | `streaming.rs`, `proxy_client.rs` on errors |

### Tool Side-Effect Events

| Event | Payload | Emitted By |
|-------|---------|-----------|
| `strategy-created-by-ai` | `{ strategy_id, name }` | `tools.rs` create_strategy tool |
| `strategy-updated-by-ai` | `{ strategy_id, name }` | `tools.rs` update_strategy tool |
| `open-promotion-modal` | `{ strategy_id, strategy_name }` | `tools.rs` promote_strategy tool |
| `open-ticket-windows` | `{ count, instrument }` | `tools.rs` open_windows tool |
| `open-chart-window` | `{ instrument, granularity }` | `tools.rs` open_windows tool |
| `update-ticket-values` | `{ instrument, direction, units, ... }` | `tools.rs` update_ticket tool |
| `start-strategy-monitor` | `{ strategy_id, timeframe, instruments }` | `tools.rs` start_monitor tool |
| `stop-strategy-monitor` | `{ watcher_id }` | `tools.rs` stop_monitor tool |

## Interfaces Consumed from Other Domains

### From oanda-trading

- `OandaClient` (via `Arc<RwLock<OandaClient>>`) -- price data, account info, candles
- `endpoints::get_candles()`, `get_account()`, `get_open_positions()`, `get_orders()`, `get_trade_history()`
- `Granularity` enum for timeframe specification

### From backtest-core

- `shared::StrategyDefinition` -- strategy validation in create/update tools
- `StrategyDefinition::from_json()` -- JSON parsing and validation
- `strategy.validate_indicator_references()` -- ensuring all indicator refs are defined

### From data-infrastructure (queries-service client)

- `QueriesServiceClient` -- HTTP client for queries-service endpoints
- Methods: `get_strategies()`, `get_strategy_by_id()`, `create_strategy()`, `update_strategy()`, `get_backtest_job()`, `list_backtest_jobs()`, `get_trades()`, `get_notes()`

### From auth-security

- Clerk JWT tokens -- passed through to queries-service for authentication
- `useDesktopAuth()` hook -- provides `getToken()` for frontend API calls

## Zero Queries Used

### Chat Messages

```typescript
// From src/queries.ts (via useUnifiedChatHistory)
export const myChatMessages = (userId?: string) =>
  zero.query.chat_messages
    .where('user_id', userId)
    .orderBy('created_at', 'asc');
```

Schema fields: `id`, `user_id`, `role`, `content`, `window_type`, `window_context`, `is_compaction`, `created_at`

### Prompt History

```typescript
// From src/queries.ts (via usePromptHistory)
export const myPromptHistory = (userId?: string) =>
  zero.query.prompt_history
    .where('user_id', userId)
    .orderBy('created_at', 'desc')
    .limit(100);
```

Schema fields: `id`, `user_id`, `content`, `created_at`

## SSE Stream Protocol

Both direct (`streaming.rs`) and proxy (`proxy_client.rs`) clients parse the same Anthropic SSE format:

```
event: message_start
data: {"message": {"id": "...", "model": "...", "usage": {"input_tokens": N, ...}}}

event: content_block_start
data: {"index": 0, "content_block": {"type": "text", "text": ""}}

event: content_block_delta
data: {"index": 0, "delta": {"type": "text_delta", "text": "chunk"}}

event: content_block_stop
data: {"index": 0}

event: message_delta
data: {"delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": N}}

event: message_stop
data: {}
```

For tool use, `content_block_start` has `{"type": "tool_use", "id": "...", "name": "..."}` and deltas carry `input_json_delta` with partial JSON that is accumulated and parsed on `content_block_stop`.

The `StreamResult` enum represents the three outcomes:
- `Complete { full_text, input_tokens, output_tokens, stop_reason }` -- normal completion
- `ToolUse { text_so_far, tool_request, input_tokens, output_tokens }` -- Claude wants to use a tool
- `Cancelled` -- user cancelled or unexpected stream end with no content
