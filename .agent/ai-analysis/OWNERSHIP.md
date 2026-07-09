# AI Analysis Domain

## Description

The AI Analysis domain owns all AI-powered features in CandleSight: Claude integration for chat, trade analysis, strategy building assistance, prompt engineering, the server-side AI proxy, context classification, and quota enforcement. It spans the Rust backend (direct and proxy Claude clients), the queries-service TypeScript (AI proxy, classifier, system prompt builder), and the React frontend (analysis UI, chat hooks, quota tracking).

## Owned Files

### Rust Backend (src-tauri/src/ai/)

```
src-tauri/src/ai/mod.rs
src-tauri/src/ai/client.rs
src-tauri/src/ai/proxy_client.rs
src-tauri/src/ai/streaming.rs
src-tauri/src/ai/strategy_builder.rs
src-tauri/src/ai/strategy_recovery.rs
src-tauri/src/ai/context.rs
src-tauri/src/ai/tools.rs
src-tauri/src/ai/prompts.rs
src-tauri/src/ai/sanitize.rs
```

### Rust Backend (src-tauri/src/analysis/)

```
src-tauri/src/analysis/mod.rs
src-tauri/src/analysis/enrichment.rs
src-tauri/src/analysis/statistics.rs
src-tauri/src/analysis/patterns.rs
src-tauri/src/analysis/trade_review.rs
src-tauri/src/analysis/performance.rs
src-tauri/src/analysis/ai_prompts.rs
```

### Rust Backend (commands)

```
src-tauri/src/commands/analysis.rs
src-tauri/src/commands/chat.rs
```

### Queries Service (TypeScript)

```
queries-service/src/anthropic-proxy.ts
queries-service/src/classifier.ts
queries-service/src/system-prompt.ts
```

### Frontend (React)

```
src/components/analysis/AnalysisFilters.tsx
src/components/analysis/DimensionBreakdown.tsx
src/components/analysis/PatternInsights.tsx
src/components/analysis/StatsDashboard.tsx
src/components/analysis/TradeList.tsx
src/components/analysis/TradesModal.tsx
src/components/analysis/TradeReviewModal.tsx
src/hooks/useTerminalChat.ts
src/hooks/useUnifiedChatHistory.ts
src/hooks/usePromptHistory.ts
src/types/ai-analysis.ts
src/lib/chatContextBuilder.ts
src/lib/aiQuotaApi.ts
src/TradeAnalysisApp.tsx
```

## Shared Files (Coordination Required)

| File | Shared With | Coordination Notes |
|------|------------|-------------------|
| `src-tauri/src/commands/chat.rs` | desktop-shell (registers commands in main.rs) | Adding new chat commands requires registration in `main.rs` invoke_handler |
| `src-tauri/src/commands/analysis.rs` | oanda-trading (uses OANDA client for candle fetching) | Analysis commands depend on OANDA endpoints for market data |
| `src-tauri/src/ai/tools.rs` | strategy-monitor, oanda-trading, backtest-core | Tools call into OANDA endpoints, queries-service, and validate against StrategyDefinition from backtest-core |
| `queries-service/src/system-prompt.ts` | queries-service routes (must be imported into Hono endpoints) | System prompt is imported by the AI route handlers |
| `queries-service/src/classifier.ts` | queries-service routes | Classifier endpoint is exposed via Hono route |
| `shared/schema.ts` | data-infrastructure | Zero schema includes `chat_messages` and `prompt_history` tables used by chat hooks |
| `src/lib/chatContextBuilder.ts` | Every window that embeds a chat terminal | Context builders must match `ChatContext` enum variants in `src-tauri/src/ai/context.rs` AND `queries-service/src/system-prompt.ts` |

## Primary Stack

- **Rust** (Tauri 2 backend): Claude API clients, SSE streaming, tool execution, input sanitization, trade analysis pipeline
- **TypeScript/Hono** (queries-service): AI proxy, prompt classification (OpenAI gpt-4o-mini), system prompt construction, quota enforcement
- **React 19** (frontend): Analysis UI components, chat hooks, context builders, quota API client

## Key Dependencies

### Rust
- `reqwest` (with TLS 1.2 minimum) - HTTP client for Anthropic API
- `tokio` / `tokio-util` / `futures-util` - Async runtime and SSE stream processing
- `serde` / `serde_json` - Serialization for API payloads and tool I/O
- `tauri` (AppHandle, Emitter, State) - Event emission and state management
- `chrono` - Time handling in tools and analysis
- `rust_decimal` - Financial precision in analysis statistics
- `shared::StrategyDefinition` - Strategy validation in create/update tools

### TypeScript (queries-service)
- `openai` - GPT-4o-mini for prompt classification
- Anthropic REST API (direct fetch, no SDK) - AI proxy passthrough
- Clerk JWKS - JWT verification for auth tokens

### Frontend
- `@tauri-apps/api/core` (invoke) - Tauri command invocation
- `@tauri-apps/api/event` (listen) - SSE event listeners for streaming
- `@rocicorp/zero/react` (useQuery) - Zero sync for chat history and prompt history
- Clerk hooks - Auth token retrieval for quota API calls

## Models Used

| Model | Usage | Where |
|-------|-------|-------|
| Claude Opus (claude-opus-4-5-20251101) | Primary chat, strategy creation, trade analysis | Default for all chat and analysis |
| Claude Sonnet (claude-sonnet-4-20250514) | Available as user selection | Selectable via model picker |
| Claude Haiku (claude-haiku-4-5-20251022) | History summarization, compaction, help command, strategy error recovery, analysis classification | Cost-optimized internal tasks |
| GPT-4o-mini (OpenAI) | Prompt classification/routing | queries-service classifier.ts only |
