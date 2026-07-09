# Desktop Shell Interfaces

## AppState Struct

Every Tauri command handler receives `AppState` via `tauri::State<'_, AppState>`. This is the single source of shared state for the entire backend.

```rust
pub struct AppState {
    pub client: Arc<tokio::sync::RwLock<OandaClient>>,
    pub config: Arc<Config>,
    pub streamer: Arc<Mutex<PriceStreamer>>,
    pub claude: Option<ClaudeClient>,
    pub watcher_handles: Arc<Mutex<HashMap<String, WatcherHandle>>>,
    pub multi_watcher_handles: Arc<Mutex<HashMap<String, MultiWatcherHandle>>>,
    pub device_manager: Arc<DeviceManager>,
    pub rate_limiter: Arc<Mutex<RateLimiter>>,
    pub credential_vault: Arc<Mutex<Option<CredentialVault>>>,
    pub candle_boundary_service: Arc<CandleBoundaryService>,
    pub wf_cancel_token: Arc<AtomicBool>,
    pub keep_monitor_in_background: Arc<AtomicBool>,
}
```

### Access Patterns:

- **Read-heavy fields** (`config`): Use `Arc` directly, config is immutable after startup
- **Read/write fields** (`client`): Use `Arc<RwLock>` for concurrent reads, exclusive writes
- **Write-heavy fields** (`streamer`, `watcher_handles`, etc.): Use `Arc<Mutex>`
- **Boolean flags** (`wf_cancel_token`, `keep_monitor_in_background`): Use `Arc<AtomicBool>` with `Ordering::SeqCst`
- **Optional state** (`credential_vault`): `Arc<Mutex<Option<T>>>` - `None` until initialized, `.lock().await` then `.as_ref()` to access

### WatcherHandle (defined in main.rs):
```rust
pub struct WatcherHandle {
    pub config_id: String,
    pub instrument: String,
    pub timeframe: String,
    pub stop_signal: Arc<AtomicBool>,
}
```

### Separate managed state:
`ChatSessionState` is registered separately via `.manage(ChatSessionState::default())` and accessed via `tauri::State<'_, ChatSessionState>` in chat commands.

## Window Management Commands (Exposed to Frontend)

All invokable from TypeScript via `invoke()`:

### Singleton window openers:
```typescript
// Open/focus the backtest window
invoke('open_backtest_window');

// Open backtest with pre-selected strategy (deep link from watcher errors)
invoke('open_backtest_window_with_strategy', { strategyId: 'abc123' });

// Open/focus analysis window
invoke('open_analysis_window');
```

### Multi-instance window openers:
```typescript
// Open chart with optional preloaded params
invoke<void>('open_chart_window', {
    instrument: 'EUR_USD',     // optional
    granularity: 'H4',         // optional
    count: 200,                // optional
    from: '2024-01-01',        // optional
    to: '2024-06-01',          // optional
    trades: '...',             // optional JSON string of trade markers
    strategyId: '...',         // optional, loads indicators
    signalDirection: 'long',   // optional
    signalId: '...',           // optional
    stopLoss: '1.12000',       // optional
    takeProfit: '1.13000',     // optional
    entryPrice: '1.12345',     // optional
    positionSize: '10000',     // optional
    indicators: '...',         // optional JSON string
});

// Open ticket window, returns count of open tickets
const ticketCount = await invoke<number>('open_ticket_window', {
    instrument: 'EUR_USD',  // optional
});

// Get current ticket window count
const count = await invoke<number>('get_ticket_window_count');

// Focus next ticket window (rotate through open tickets)
const focusedLabel = await invoke<string | null>('focus_next_ticket_window', {
    currentLabel: 'ticket-12345',  // optional
});
```

### Startup and lifecycle:
```typescript
// Open configured startup windows after login
invoke('open_startup_windows', {
    windows: ['account', 'watcher', 'backtest']
});

// Open login window (used after logout)
invoke('open_login_window', { fromLogout: true });

// Close login window after successful auth
invoke('close_login_window');

// Emit sign-out event to close all windows (used during logout)
invoke('close_all_windows');
```

### Notifications:
```typescript
// Send test notification
invoke<boolean>('test_notification');

// Enable/disable desktop notifications
invoke('set_desktop_notifications_enabled', { enabled: true });
```

### Pending matches:
```typescript
// Get pattern matches emitted while frontend was not listening
const matches = await invoke<object[]>('get_pending_matches');

// Clear all pending matches
invoke('clear_pending_matches');

// Remove specific match
invoke('remove_pending_match', { matchId: 'abc123' });
```

### Background mode:
```typescript
// Control whether monitors keep running when watcher window closes
invoke('set_keep_monitor_in_background', { enabled: true });
```

## Command Registration Interface

Other domains register their commands through this process:

1. Create a new module file in `src-tauri/src/commands/` (e.g., `mydomain.rs`)
2. Add `pub mod mydomain;` to `src-tauri/src/commands/mod.rs`
3. In `main.rs`, add import:
   ```rust
   use commands::mydomain::{my_command_a, my_command_b};
   ```
4. Add to the `invoke_handler` list in `main.rs`:
   ```rust
   .invoke_handler(tauri::generate_handler![
       // ... existing commands ...
       my_command_a,
       my_command_b,
   ])
   ```

Commands that need shared state access their dependencies through function parameters:
- `state: tauri::State<'_, AppState>` - for app state
- `app_handle: AppHandle` - for emitting events, getting windows
- `chat_state: tauri::State<'_, ChatSessionState>` - for chat state

## Event Emission Patterns

Events are the primary way the backend communicates asynchronous data to the frontend.

### From Rust:
```rust
// Emit to all windows
app_handle.emit("event-name", payload)?;

// Emit to specific window
if let Some(window) = app_handle.get_webview_window("watcher") {
    window.emit("event-name", payload)?;
}
```

### From Frontend:
```typescript
import { listen } from '@tauri-apps/api/event';

const unlisten = await listen<PayloadType>('event-name', (event) => {
    console.log(event.payload);
});

// Cleanup
unlisten();
```

### Key events emitted by desktop-shell:

| Event | Payload | Source | Purpose |
|-------|---------|--------|---------|
| `sign-out` | `()` | `close_all_windows` | Tell all windows to close themselves |
| `oauth-callback` | `{ code, state }` | Deep link handler | Complete OAuth flow |
| `check-for-updates` | `()` | Menu bar | Trigger update check in frontend |
| `debug-log` | `{ message }` | Windows single-instance handler | Debug deep link processing |
| `tray-show-window` | `()` | Tray fallback | Ask frontend to show a window |

### Events consumed by desktop-shell (frontend):
- `sign-out` - Each window listens and closes itself
- `check-for-updates` - App.tsx triggers UpdateModal
- `oauth-callback` - Login flow processes code/state

### Critical rule: Listeners before streams
Event listeners must be set up BEFORE starting the command that produces events. Otherwise, early events are lost. See `docs/tauri-guide.md` for the correct pattern.

### Remote API access requirement
In production builds (localhost plugin), events only reach the frontend if `capabilities/default.json` includes:
```json
"remote": {
    "urls": ["http://localhost:*", "http://127.0.0.1:*"]
}
```
Without this, `app_handle.emit()` succeeds silently but the frontend never receives the event.

## Configuration Interface

### Config struct (read-only after startup):
```rust
pub struct Config {
    pub api_key: Option<String>,           // OANDA API key (set after vault unlock)
    pub account_id: Option<String>,        // OANDA account ID (set after vault unlock)
    pub environment: OandaEnvironment,     // Practice or Live
    pub database_url: Option<String>,      // PostgreSQL connection
    pub anthropic_api_key: Option<String>, // AI features (None = disabled)
    pub build_mode: BuildMode,             // Dev, Staging, or Prod
    pub queries_service_url: Option<String>, // Backend service URL
}
```

### BuildMode enum:
```rust
pub enum BuildMode {
    Dev,      // Default, local development
    Staging,  // Pre-production testing
    Prod,     // Production release
}
```

### OandaEnvironment enum:
```rust
pub enum OandaEnvironment {
    Practice,  // api-fxpractice.oanda.com
    Live,      // api-fxtrade.oanda.com
}
```

### Loading priority:
Runtime `.env` > Compile-time `option_env!()` > Default values

### Error type:
```rust
pub enum Error {
    Config(String),
    Http(reqwest::Error),
    Json(serde_json::Error),
    OandaApi(String),
    Api(String),
    Env(std::env::VarError),
    Io(std::io::Error),
    Database(String),
    InvalidArgument(String),
    Strategy(String),
    Crypto(String),
    Auth(String),
}
```

This enum is used within the library crate. At the Tauri command boundary, all errors are converted to `String` via `.map_err(|e| e.to_string())`.
