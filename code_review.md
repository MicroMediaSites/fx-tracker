# Code Review: FX Tracker (main branch)

**Date:** December 2, 2025
**Reviewer:** Claude (Senior Engineer Perspective)
**Branch:** main
**Commit:** 949e5ad (M6 - position management)

---

## Executive Summary

This is a **well-structured foundation** but has **significant gaps** that need addressing before this can be considered production-ready for a financial application. The architecture is sound, but there are critical security concerns, violations of your own engineering principles, and a complete absence of tests despite documented test strategy.

---

## CRITICAL SECURITY ISSUES

### 1. CSP Disabled in Production Configuration

**File:** `src-tauri/tauri.conf.json:25-26`

```json
"security": {
  "csp": null
}
```

**Problem:** Content Security Policy is completely disabled. This opens the app to XSS attacks if any untrusted content ever reaches the WebView. For a financial application handling trading data, this is unacceptable.

**Fix:** Implement restrictive CSP:

```json
"security": {
  "csp": "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self' https://api-fxpractice.oanda.com https://api-fxtrade.oanda.com https://stream-fxpractice.oanda.com https://stream-fxtrade.oanda.com"
}
```

### 2. No Input Validation on Order Placement

**File:** `src-tauri/src/main.rs:148-196`

The `place_order` command accepts `instrument: String` and `units: i64` with **zero validation**:

- No check that `instrument` is a valid OANDA instrument format
- No check for reasonable unit bounds (could pass `i64::MAX`)
- No sanitization of the instrument string before sending to API

**Risk:** Malformed requests could cause unexpected behavior or API errors that leak information.

### 3. API Key Stored in Plain Text Memory

**File:** `src-tauri/src/oanda/client.rs:15-24`

```rust
pub struct OandaClient {
    api_key: String,  // Plain text in memory
```

The API key remains in memory as a plain `String` for the lifetime of the application. In a memory dump or debug scenario, this could be extracted.

**Recommendation:** Consider using `secrecy::Secret<String>` to zeroize on drop and prevent accidental logging.

### 4. Sensitive Data in Error Messages

**File:** `src-tauri/src/oanda/endpoints.rs:111-117`

```rust
eprintln!("Failed to parse positions response: {}", e);
eprintln!("Raw response: {}", &text[..text.len().min(1000)]);
```

Raw API responses (which may contain account details, balances, trade info) are logged to stderr. This could leak sensitive financial data in logs.

### 5. No TLS Certificate Validation Configuration

**File:** `src-tauri/src/oanda/client.rs:38-41`

```rust
let client = Client::builder()
    .build()?;
```

The reqwest client is built with defaults. For a financial application, you should explicitly configure:

- Certificate pinning for OANDA's certificates
- Minimum TLS version (1.2+)
- Disable older cipher suites

### 6. Streaming Connection Not Secured Against Reconnection Attacks

**File:** `src-tauri/src/oanda/streaming.rs:95-180`

If the stream disconnects, there's no reconnection logic with exponential backoff. An attacker could potentially force disconnections and the app would silently stop receiving price updates without user notification.

---

## ENGINEERING PRINCIPLES VIOLATIONS

### 1. Comments in Code (Violates Principle #2 "Never add comments")

Multiple files have extensive doc comments and inline comments:

- `src-tauri/src/oanda/client.rs`: Lines 1-5, 27-37, 60-68, etc.
- `src-tauri/src/oanda/endpoints.rs`: Lines 1-5, 11-33, etc.
- `src-tauri/src/oanda/types.rs`: Nearly every struct has doc comments

Your engineering principles state: **"Never add comments - code should be self-explanatory"**

The doc comments (`///`) are arguably useful for library documentation, but the inline explanatory comments violate the principle.

### 2. Hardcoded CSS Values (Violates Principle #3 "CSS Architecture")

**File:** `src/routes/trade/+page.svelte`

You're using Tailwind with CSS variables which is fine, but several hardcoded values exist:

```svelte
class="text-2xl font-bold"  // Line 94
class="text-xl font-semibold"  // Line 89
class="text-sm font-medium"  // Line 96
```

These should reference design tokens for consistency.

### 3. Floating Point for Financial Calculations (Precision Risk)

**File:** `src-tauri/src/oanda/streaming.rs:36-42`

```rust
let bid_f: f64 = bid.parse().unwrap_or(0.0);
let ask_f: f64 = ask.parse().unwrap_or(0.0);
let spread = if bid_f > 0.0 && ask_f > 0.0 {
    format!("{:.5}", ask_f - bid_f)
```

You correctly use `rust_decimal::Decimal` in models, but here you're using `f64` for spread calculation. Floating point arithmetic can introduce precision errors in financial calculations.

**Also in frontend:** `src/lib/stores/prices.ts:39-42`:

```typescript
const prevBid = parseFloat(prev.bid);
const currBid = parseFloat(price.bid);
```

### 4. Default Export Used

**File:** `src/lib/stores/prices.ts:119-126`

```typescript
export const prices = {
  subscribe: pricesMap.subscribe,
  // ...
};
```

This is a named export which is good, but you're exporting an object that acts like a default. The pattern is inconsistent with your principle of avoiding default exports.

---

## TEST STRATEGY VIOLATIONS

### 1. Zero Tests Despite Documented Strategy

**File:** `docs/testing/test-coverage-status.md`

States: "M1 & M2 Complete: Core functionality working but untested"

Your test strategy document specifies:

- "No new features without tests - Starting from M3"
- "Minimum 70% coverage for new modules"

But there are **zero** tests in the codebase. The `config.rs` tests mentioned in the architecture doc exist (lines 104-136), but that's 2 unit tests total.

### 2. Missing Test Configuration Files

The testing guide references these files that don't exist:

- `vitest.config.ts` - Not present
- `vitest.setup.ts` - Not present
- Test scripts in `package.json` - Not present

### 3. No Error Path Testing

Your test strategy states: "Test error cases - Not just happy path"

With zero tests, obviously no error paths are tested. The error handling code paths in `endpoints.rs` are completely untested.

---

## PERFORMANCE & SCALABILITY CONCERNS

### 1. No Rate Limiting

**File:** `src-tauri/src/oanda/endpoints.rs`

Every API call goes directly to OANDA with no rate limiting. If the frontend rapidly calls `get_account`, `get_positions`, `get_orders` (as it does on dashboard load), you could hit OANDA's rate limits.

### 2. Inefficient Map Recreation on Every Price Update

**File:** `src/lib/stores/prices.ts:77`

```typescript
return new Map(map); // Create new map to trigger reactivity
```

On every price tick (potentially multiple times per second), you're creating an entirely new Map. This is inefficient. Consider using Svelte 5's fine-grained reactivity instead.

### 3. No Caching of Static Data

Account information, instrument lists, etc. are fetched fresh every time. Consider caching with TTL for data that doesn't change frequently.

### 4. Blocking Mutex in Streaming

**File:** `src-tauri/src/main.rs:247-251`

```rust
let streamer = state.streamer.lock().await;
streamer
    .start(instruments, app_handle)
    .await
```

The mutex is held across an async operation. This blocks other streaming commands during the start operation. Use a more granular locking strategy.

---

## MAINTAINABILITY ISSUES

### 1. Type Duplication

You have three layers of types:

1. `OandaTrade`, `OandaPosition` (OANDA API types) in `types.rs`
2. `Trade`, `Position` (internal models) in `models/`
3. `Trade`, `Position` (frontend types) in `main.rs`
4. TypeScript interfaces in `src/lib/types/index.ts`

Four representations of the same data. This violates DRY and creates maintenance burden. Changes to the OANDA API require updates in 4 places.

### 2. Inconsistent Error Handling Patterns

Some endpoints use:

```rust
.error_for_status()?;
```

Others manually check:

```rust
if text.contains("errorMessage") || text.contains("errorCode") {
    return Err(...)
}
```

Pick one pattern and use it consistently.

### 3. Magic Strings Throughout

**File:** `src-tauri/src/oanda/types.rs:206-215`

```rust
order_type: "MARKET".to_string(),
time_in_force: "FOK".to_string(),
position_fill: "DEFAULT".to_string(),
```

Use enums or constants for these values.

### 4. Dead Code (Stub Modules)

The following modules exist but are empty:

- `src-tauri/src/db/mod.rs`
- `src-tauri/src/analysis/performance.rs`
- `src-tauri/src/analysis/metrics.rs`
- `src-tauri/src/backtest/engine.rs`
- `src-tauri/src/backtest/strategy.rs`

Either implement them or remove them. Dead code is confusing.

---

## BUG RISKS

### 1. Silent Failures in Price Streaming

**File:** `src-tauri/src/oanda/streaming.rs:159-161`

```rust
Err(e) => {
    eprintln!("Failed to parse stream message: {} - Line: {}", e, line);
}
```

Parse failures are silently logged but don't notify the frontend. The UI could show stale prices without the user knowing.

### 2. Race Condition in Streaming State

**File:** `src-tauri/src/oanda/streaming.rs:100-108`

```rust
if self.is_running() {
    return Err(Error::OandaApi("Stream already running".to_string()));
}
self.running.store(true, Ordering::SeqCst);
```

There's a TOCTOU (time-of-check-time-of-use) race between `is_running()` check and `store(true)`. Two concurrent calls could both pass the check.

### 3. Unhandled Edge Case in Position Display

**File:** `src-tauri/src/main.rs:33-43`

```rust
impl From<models::Position> for Position {
    fn from(p: models::Position) -> Self {
        Self {
            units: p.units.to_string(),
```

If `units` is `0` (flat position), this still converts and displays. The frontend doesn't filter out flat positions, potentially showing confusing zero-unit positions.

### 4. Unchecked Unwrap on Stream Price

**File:** `src-tauri/src/oanda/streaming.rs:32-33`

```rust
let bid = price.bids.first().map(|b| b.price.clone()).unwrap_or_default();
let ask = price.asks.first().map(|a| a.price.clone()).unwrap_or_default();
```

If `bids` or `asks` is empty (which can happen during market close), you get empty strings that parse to `0.0`. The spread calculation then produces misleading results.

---

## REQUIRED CHANGES (Priority Order)

### Critical (Do Before Any Production Use)

| # | Issue | File | Line(s) |
|---|-------|------|---------|
| 1 | Enable restrictive CSP | `tauri.conf.json` | 25-26 |
| 2 | Add input validation for `place_order` | `main.rs` | 148-196 |
| 3 | Remove raw response logging | `endpoints.rs` | 111-117, 143-146, 173-176, 296-299 |
| 4 | Add TLS configuration | `client.rs` | 38-41 |
| 5 | Add rate limiting | `endpoints.rs` | All API calls |

### High Priority (Before Beta)

| # | Issue | File | Line(s) |
|---|-------|------|---------|
| 6 | Create test infrastructure | N/A | N/A |
| 7 | Add tests for `config.rs` loading edge cases | `config.rs` | 74-101 |
| 8 | Add tests for API endpoint error handling | `endpoints.rs` | All |
| 9 | Fix TOCTOU race in streaming | `streaming.rs` | 100-108 |
| 10 | Use Decimal for spread calculation | `streaming.rs` | 36-42 |
| 11 | Notify frontend of stream errors | `streaming.rs` | 159-161 |

### Medium Priority (Technical Debt)

| # | Issue | File | Line(s) |
|---|-------|------|---------|
| 12 | Remove or implement stub modules | `db/`, `analysis/`, `backtest/` | All |
| 13 | Consolidate error handling patterns | `endpoints.rs` | Various |
| 14 | Replace magic strings with enums | `types.rs` | 206-215 |
| 15 | Add reconnection logic for streaming | `streaming.rs` | 95-180 |
| 16 | Optimize Map recreation in price store | `prices.ts` | 77 |

### Low Priority (Polish)

| # | Issue | File | Line(s) |
|---|-------|------|---------|
| 17 | Remove verbose doc comments (per your principles) | Various | Various |
| 18 | Add design tokens for hardcoded Tailwind values | `+page.svelte` | Various |
| 19 | Consider `secrecy::Secret` for API key | `client.rs` | 15-24 |
| 20 | Add caching layer for infrequently-changing data | N/A | N/A |

---

## Summary

### What's Good

- Clean architecture with proper separation of concerns
- Correct use of `rust_decimal` for financial precision (mostly)
- Type-safe IPC with Tauri commands
- Good error type design with `thiserror`
- Svelte 5 runes used correctly

### What's Concerning

- Zero test coverage for a financial application
- CSP disabled
- No input validation on trading endpoints
- Sensitive data logged
- Several race conditions and edge cases not handled

### Verdict

This codebase is a solid **prototype/MVP** but is not production-ready for handling real money. The security issues need immediate attention, and the complete lack of tests is a significant risk for a trading application where bugs can mean financial loss.
