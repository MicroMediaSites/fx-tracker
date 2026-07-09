# AI Analysis Bugs

<!-- Template for new entries:

## BUG-AIXX: Short description
- **Status**: open | investigating | fixed
- **Severity**: critical | high | medium | low
- **Found**: YYYY-MM-DD
- **Fixed**: YYYY-MM-DD (if applicable)
- **Files**: list of affected files
- **Description**: What is broken and how to reproduce
- **Root Cause**: Why it happens (if known)
- **Fix**: What was done to fix it (if fixed)

-->

## BUG-AI01: Watcher AI chat context shows undefined for monitor state (BUG-033)
- **Status**: fixed
- **Severity**: high
- **Found**: 2026-01-24
- **Fixed**: 2026-03-01
- **Files**: `queries-service/src/system-prompt.ts`
- **Description**: AI chat in the Watcher window showed "undefined" for strategy names, empty instruments/timeframe, and "pending" for all entry prices even when monitors were actively running. The AI had no awareness of what the user was monitoring.
- **Root Cause**: Three-location field name sync issue. Rust nested structs (`WatcherInfo`, `SignalInfo`, `ParameterInfo`, `TradeAIScore`, `IndicatorAnalysis`) use `#[serde(rename_all = "camelCase")]` which serializes `strategy_name` as `strategyName` in JSON. The frontend `chatContextBuilder.ts` correctly uses camelCase. But `queries-service/src/system-prompt.ts` used snake_case in TypeScript interfaces and `describeContext()` function, so all nested field accesses returned `undefined`.
- **Fix**: Updated all nested struct field names in system-prompt.ts to match Rust's camelCase serialization:
  - WatcherContext: `strategy_name` -> `strategyName`, `entry_price` -> `entryPrice`
  - BacktestingContext: `current_value` -> `currentValue`, `default_value` -> `defaultValue`
  - TradeReviewContext: `risk_management` -> `riskManagement`, `supported_trade` -> `supportedTrade`, `at_entry` -> `atEntry`, `at_exit` -> `atExit`
- **Prevention**: When adding new nested structs with `#[serde(rename_all = "camelCase")]` in Rust context.rs, verify the same camelCase field names are used in ALL three locations: (1) Rust struct definition, (2) frontend chatContextBuilder.ts, (3) queries-service system-prompt.ts. Note: top-level enum variant fields remain snake_case because the ChatContext enum itself does NOT have `rename_all`.
