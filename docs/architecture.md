# CandleSight Architecture (HISTORICAL)

> **This document predates the CandleSight→wickd local-first conversion
> (AGT-635..653) and is kept for historical context only.** The Zero sync
> layer, Clerk authentication, queries-service, PostgreSQL, and the Railway
> deployment it describes were all removed. The current architecture is a
> local-first Tauri app reading `~/.wickd/app.db` — see `docs/local-store.md`
> and `CLAUDE.md`.


## Project Vision

A desktop trading application that replaces OANDA's interface for daily trading:

1. **View & manage** positions and orders in real-time
2. **Execute trades** directly from the app
3. **Analyze** trade history and performance
4. **Backtest** strategies against historical data
5. **Paper trade** strategies before going live
6. **Automate** strategy execution (future)

---

## Architecture Overview

### Current State (v2 - React + Zero)

```
┌─────────────────────────────────────────────────────────────┐
│                    Tauri Desktop App                        │
├─────────────────────────────────────────────────────────────┤
│  ┌───────────────────────────────────────────────────────┐  │
│  │                 React Frontend                        │  │
│  │  • Pages: Dashboard, Trade, History, Backtest         │  │
│  │  • Zero client (local-first sync)                     │  │
│  │  • Clerk authentication                               │  │
│  │  • IndexedDB cache (offline support)                  │  │
│  └────────────┬─────────────────────────┬────────────────┘  │
│               │ Tauri IPC               │ WebSocket         │
│  ┌────────────▼──────────────┐          │                   │
│  │      Rust Backend         │          │                   │
│  │  • OANDA API client       │          │                   │
│  │  • Order execution        │          │                   │
│  │  • Price streaming        │          │                   │
│  └────────────┬──────────────┘          │                   │
└───────────────┼─────────────────────────┼───────────────────┘
                │                         │
                ▼                         ▼
          ┌───────────┐         ┌─────────────────────┐
          │ OANDA API │         │    Railway Cloud    │
          └───────────┘         │  ┌───────────────┐  │
                                │  │  zero-cache   │  │
                                │  └───────┬───────┘  │
                                │          │          │
                                │  ┌───────▼───────┐  │
                                │  │ queries-svc   │  │
                                │  └───────┬───────┘  │
                                │          │          │
                                │  ┌───────▼───────┐  │
                                │  │  PostgreSQL   │  │
                                │  └───────────────┘  │
                                └─────────────────────┘
```

---

## Data Flow

### OANDA Data (Real-time Trading)

```
Frontend                    Rust Backend                OANDA
────────                    ────────────                ─────
invoke("get_account")  ──>  get_account()         ──>  GET /v3/accounts/{id}
                       <──  Account struct        <──  JSON response

invoke("start_stream")
listen("price-update") ──>  PriceStreamer         ──>  GET /pricing/stream
emit("price-update")   <──  parse + emit          <──  line-delimited JSON
```

### Zero Data (Persistence + Sync)

```
Frontend                    Zero Client                 Railway
────────                    ───────────                 ───────
useQuery(myTrades)     ──>  IndexedDB (local)
                       <──  instant response

zero.mutate.note.insert()
                       ──>  optimistic update (local)
                       ──>  WebSocket to zero-cache  ──>  queries-service
                                                     ──>  PostgreSQL
                       <──  confirmation             <──  WAL broadcast
```

### Combined Flow

```
1. App Start
   └─> Clerk auth → get user ID
   └─> Zero connects to zero-cache
   └─> Rust backend ready

2. Dashboard Load
   └─> Zero query: local trades/notes (instant)
   └─> Tauri invoke: live positions from OANDA
   └─> Background: sync OANDA trades → Zero

3. Add Note to Trade
   └─> Zero mutate (optimistic, instant UI)
   └─> Syncs to PostgreSQL via zero-cache
   └─> Available on other devices

4. Execute Trade
   └─> Tauri invoke → Rust → OANDA API
   └─> On success: sync trade to Zero
```

---

## Technical Stack

| Layer | Technology | Purpose |
|-------|------------|---------|
| **Desktop Framework** | Tauri 2.0 | Native app shell, IPC bridge |
| **Frontend** | React 19 + TypeScript | UI components |
| **Styling** | Tailwind CSS v4 | Utility-first styling |
| **Build Tool** | Vite | Fast bundling |
| **Routing** | wouter | Lightweight routing |
| **Local-First Sync** | @rocicorp/zero | Offline-first persistence |
| **Authentication** | Clerk | User auth + JWT |
| **Cloud Database** | PostgreSQL (Railway) | Source of truth |
| **Sync Server** | zero-cache (Railway) | Real-time sync hub |
| **Query Processor** | queries-service (Hono) | Mutation validation |
| **Backend** | Rust | OANDA integration, PostgreSQL sync |
| **HTTP Client** | reqwest, sqlx | OANDA REST API, PostgreSQL |
| **Async Runtime** | tokio | Async operations, streaming |

---

## Type Layer Architecture

Two-layer type system separates external API concerns from domain logic:

```
┌─────────────────────────────────────────────────────────────────┐
│                        OANDA Types                              │
│              (src-tauri/src/oanda/types.rs)                     │
│                                                                 │
│  • Matches OANDA API JSON exactly                               │
│  • All numbers as strings (API format)                          │
│  • String-based enums ("OPEN", "MARKET", etc.)                  │
│  • Used only for API serialization/deserialization              │
└────────────────────────────┬────────────────────────────────────┘
                             │ From<OandaType> for DomainType
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                       Domain Models                             │
│                (src-tauri/src/models/*.rs)                      │
│                                                                 │
│  • Rust-idiomatic types                                         │
│  • rust_decimal::Decimal for all money/prices                   │
│  • chrono::DateTime<Utc> for timestamps                         │
│  • Type-safe enums (TradeState, OrderType, etc.)                │
│  • Used throughout application logic                            │
└─────────────────────────────────────────────────────────────────┘
                             │
                             ▼ (v2 addition)
┌─────────────────────────────────────────────────────────────────┐
│                       Zero Schema                               │
│                      (schema.ts)                                │
│                                                                 │
│  • Shared across frontend and queries-service                   │
│  • Numbers as strings (for Decimal precision)                   │
│  • Timestamps as numbers (milliseconds)                         │
│  • Type-safe queries and mutations                              │
└─────────────────────────────────────────────────────────────────┘
```

---

## Project Structure

```
fx-tracker/
├── src-tauri/              # Rust backend
│   ├── src/
│   │   ├── main.rs         # Tauri entry point + commands
│   │   ├── lib.rs          # Library root
│   │   ├── config.rs       # Environment configuration
│   │   ├── error.rs        # Error types
│   │   ├── db.rs           # PostgreSQL sync (NEW)
│   │   ├── oanda/          # OANDA API client
│   │   └── models/         # Domain models
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   └── .env                # OANDA creds, DATABASE_URL
│
├── src/                    # React frontend
│   ├── ZeroContext.tsx     # Zero provider + hooks
│   ├── App.tsx             # Main app component
│   ├── main.tsx            # Entry point
│   └── index.css           # Tailwind styles
│
├── queries-service/        # Zero query processor (Railway)
│   ├── src/
│   │   ├── index.ts        # Hono server
│   │   └── mutators.ts     # Mutation handlers
│   └── package.json
│
├── schema.ts               # Shared Zero schema (new)
├── schema.cjs              # Compiled for zero-cache
│
└── docs/
    ├── architecture.md
    ├── migration-plan.md
    └── ...
```

---

## Zero Schema (v2)

```typescript
// schema.ts - shared across frontend and queries-service

const trade = table('trade')
  .columns({
    id: string(),              // OANDA trade ID
    user_id: string(),         // Clerk user ID
    instrument: string(),
    units: string(),           // Decimal as string
    open_price: string(),
    close_price: string().optional(),
    realized_pl: string().optional(),
    state: string(),
    synced_at: number(),
  })
  .primaryKey('id');

const note = table('note')
  .columns({
    id: string(),
    user_id: string(),
    trade_id: string().optional(),
    title: string(),
    content: string(),
    created_at: number(),
    updated_at: number(),
  })
  .primaryKey('id');

const strategy = table('strategy')
  .columns({
    id: string(),
    user_id: string(),
    name: string(),
    description: string(),
    indicators: string(),      // JSON: IndicatorDefinition[]
    entry_rules: string(),     // JSON: EntryRule[]
    entry_logic: string(),     // JSON: EntryLogic
    exit_rules: string(),      // JSON: ExitRule[]
    risk_settings: string(),   // JSON: RiskSettings
    version: number(),
    is_active: boolean(),
    created_at: number(),
    updated_at: number(),
  })
  .primaryKey('id');

const strategy_config = table('strategy_config')
  .columns({
    id: string(),
    strategy_id: string(),
    user_id: string(),
    name: string(),
    instrument: string(),
    timeframe: string(),
    indicator_params: string(),
    risk_overrides: string().optional(),
    is_live: boolean(),
    created_at: number(),
    updated_at: number(),
  })
  .primaryKey('id');
```

---

## Deployment

### Development

```bash
# Terminal 1: zero-cache (local)
npx zero-cache dev

# Terminal 2: queries-service
cd queries-service && npm run dev

# Terminal 3: Tauri dev
npm run tauri dev
```

### Production

```
Railway:
├── PostgreSQL (managed)
├── zero-cache (Node.js service)
└── queries-service (Node.js service)

User's Mac:
└── FX Tracker.app (Tauri build)
```

---

## Security Considerations

- **OANDA API key**: Encrypted in local vault, never exposed to frontend
- **Clerk JWT**: Used for Zero auth, validated in queries-service
- **PostgreSQL**: Row-level isolation via user_id filtering
- **Zero sync**: All mutations validated server-side before persistence

---

## Credential Vault Architecture

OANDA credentials are stored using zero-knowledge encryption - the server never sees plaintext credentials.

### Encryption Flow

```
┌─────────────────────────────────────────────────────────────────┐
│                         USER DEVICE                              │
│                                                                  │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────────┐  │
│  │  User enters │───▶│  Derive Key  │───▶│ Encrypt/Decrypt  │  │
│  │  Master Pass │    │  (Argon2id)  │    │   (AES-256-GCM)  │  │
│  └──────────────┘    └──────────────┘    └────────┬─────────┘  │
│                                                    │            │
│         Password NEVER leaves device               │            │
│         Key NEVER leaves device                    │            │
└────────────────────────────────────────────────────┼────────────┘
                                                     │
                                          Encrypted blob only
                                                     │
                                                     ▼
                                              Zero / PostgreSQL
```

### Cryptographic Components

| Component | Algorithm | Purpose |
|-----------|-----------|---------|
| Key Derivation | Argon2id (128MB) | Derive encryption key from master password |
| Encryption | AES-256-GCM | Authenticated encryption of credentials |
| Subkey Derivation | HKDF | Separate keys for practice/live/HMAC |
| Salt | 16 bytes random | Unique per user, prevents rainbow tables |
| Nonce | 12 bytes random | Fresh random nonce on every encrypt |
| Tamper Protection | HMAC-SHA256 | Detect database tampering |

### Key Architecture Decisions

1. **Master Password**: 16 character minimum (not "passphrase")
2. **Per-Device Encryption**: Each device has uniquely encrypted blob
3. **Zero-Knowledge**: Client-side encryption, server never sees plaintext
4. **Memory Security**: `secrecy`/`zeroize` crates, keys only in Rust
5. **Deferred Validation**: Encrypt first, validate OANDA credentials on use
6. **Rate Limiting**: 5 failed decryption attempts = 5 minute lock
7. **Session Timeout**: Auto-lock after 5 minutes inactivity

### Database Schema

```sql
CREATE TABLE user_credentials (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    encrypted_blob TEXT NOT NULL,  -- base64(version || salt || kdf_params || nonce || ciphertext || tag)
    hmac TEXT NOT NULL,            -- base64(HMAC-SHA256)
    created_at INTEGER NOT NULL,
    UNIQUE(user_id, device_id)
);
```

---

## Real-Time Streaming Architecture

Since AGT-652 the app is a **client of the wickd stream hub** — it holds no
unconditional OANDA subscription of its own. `candlesight_lib::hub_stream`
(src-tauri/src/hub_stream.rs) supervises the feed:

1. **Attach**: probe `~/.wickd/stream.sock` (the `wickd stream` socket hub)
   and re-emit its NDJSON lines as Tauri events.
2. **Degrade to direct**: an instrument the hub isn't observed streaming
   falls back to a direct `PriceStreamer` subscription (the CLI watcher's
   own semantics).
3. **Host**: when no hub answers, the app binds the hub itself and publishes
   byte-identical NDJSON (`wickd_core::ndjson::event_line`) so CLI consumers
   attach to the app — the machine always holds ONE upstream subscription
   per hub-covered instrument.

Prices remain ephemeral in a Zustand store; persisted domains live in the
local SQLite store (`docs/local-store.md`).

### Data Flow

```
wickd stream (hub) ──┐
                     ├→ hub_stream supervisor → Tauri Event → Zustand Store → React Components
OANDA (direct        │                                             ↓
 fallback/host) ─────┘                                prices: Map<instrument, PriceUpdate>
                                                                   ↓
                                                     Selector-based subscriptions
```

### Zustand Store Design

```typescript
interface PriceStore {
  prices: Record<string, PriceUpdate>;
  updatePrice: (price: PriceUpdate) => void;
  streaming: boolean;
  setStreaming: (streaming: boolean) => void;
}

// Components subscribe with selectors for minimal re-renders
const price = usePriceStore((state) => state.prices['EUR_USD']);
```

### Performance

- Store updates: Thousands/second (no issue)
- Leaf component re-renders: 2-5ms each
- No cascading re-renders (selector isolation)
- No database writes for price ticks

### Key Files

- `src/stores/priceStore.ts` - Zustand store
- `src/hooks/usePriceStream.ts` - Tauri event bridge
- `src-tauri/src/hub_stream.rs` - hub-first streaming supervisor (AGT-652)
- `crates/wickd-core/src/stream_hub.rs` / `hub_client.rs` - the shared hub contract

## One Watcher Engine (AGT-652)

The desktop app hosts **no watcher engine**. `wickd watch` (typically
launchd-supervised) is the single strategy runtime; the app's Live Monitor
window is a read-only client of its outputs:

| Daemon output | Path (WICKD_HOME-aware) | App command |
|---|---|---|
| Signal feed | `~/.wickd/alert-queue.ndjson` | `daemon_queue_list` |
| Pending proposals | `~/.wickd/pending.json` | `daemon_pending_list` |
| Liveness | process table (`wickd* watch …`) | `daemon_status` |

Approval of a pending signal is deliberately NOT an app action — it is the
CLI's `wickd approve <id>` (the semi-auto trust ladder). The app renders the
queue and offers the command to copy.
