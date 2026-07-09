# Desktop Shell Architecture

## Application Startup Sequence

The app launches through `main()` in `main.rs` (annotated `#[tokio::main]`). The sequence is:

1. **Initialize logging** - `tracing_subscriber` with INFO level
2. **Load config** - `Config::load_with_compile_time()` tries multiple .env locations, then falls back to compile-time `option_env!()` values baked into the binary during CI builds
3. **Create OANDA client** - `OandaClient::new(&config)` with whatever credentials are available (may be empty, updated after vault unlock)
4. **Create CandleBoundaryService** - For streaming-based candle close detection
5. **Create PriceStreamer** - Initialized with empty/default credentials, linked to candle boundary service and optional spread stats collector
6. **Initialize Claude client** - Only if `ANTHROPIC_API_KEY` is set (note: never embedded at compile time for security; production routes through queries-service proxy)
7. **Build Tauri app** with plugin chain, setup closure, state management, command registration
8. **Run the app** with lifecycle event handler

### Inside the setup closure (runs once at app start):

1. **Production URL redirect** - In release builds, navigates the default login window from `tauri://` to `http://localhost:14201/index.html?window=login` so Clerk auth works
2. **Windows size adjustment** - Resizes login window to 500x670 to account for WebView2 rendering differences
3. **macOS magnification disable** - Sets `allowsMagnification = false` on WKWebView via Objective-C runtime (BUG-040)
4. **Menu bar construction** - App, File, Edit, View, Window menus with keyboard shortcuts
5. **OAuth configuration** - Loads `CLERK_OAUTH_CLIENT_ID`, `DESKTOP_AUTH_WEB_URL`, `CLERK_FRONTEND_API_URL` from env/compile-time (fails hard if missing, AUDIT-002)
6. **Deep link registration** - `candlesight://` scheme handler for OAuth callbacks (macOS + Windows)
7. **Dev callback server** - Debug-only HTTP server for OAuth callbacks since deep links do not work in `tauri dev`
8. **System tray setup** - Always enabled for all users

### After build, the `.run()` handler manages lifecycle:

- **macOS Reopen** - When dock icon clicked with no visible windows, focuses watcher window
- **Watcher window close** - If `keep_monitor_in_background` is false, stops all watchers before window closes
- **macOS ExitRequested** - Prevents app exit to stay in tray (macOS only; Windows users expect close = quit, BUG-055)

## The AppState Struct

`AppState` is the central shared state, registered via `.manage()` and injected into every command handler as `tauri::State<'_, AppState>`.

```rust
pub struct AppState {
    pub client: Arc<tokio::sync::RwLock<OandaClient>>,       // OANDA API client, RwLock because credentials change after vault unlock
    pub config: Arc<Config>,                                   // Immutable after startup
    pub streamer: Arc<Mutex<PriceStreamer>>,                   // Price streaming, mutable for subscribe/unsubscribe
    pub claude: Option<ClaudeClient>,                          // AI client, None if no API key
    pub watcher_handles: Arc<Mutex<HashMap<String, WatcherHandle>>>,       // Single-instrument watcher controls
    pub multi_watcher_handles: Arc<Mutex<HashMap<String, MultiWatcherHandle>>>, // Multi-instrument watcher controls
    pub device_manager: Arc<DeviceManager>,                    // Crypto device identity
    pub rate_limiter: Arc<Mutex<RateLimiter>>,                // Vault unlock rate limiting
    pub credential_vault: Arc<Mutex<Option<CredentialVault>>>, // None until master password unlocks it
    pub candle_boundary_service: Arc<CandleBoundaryService>,  // Detects candle closes from streaming data
    pub wf_cancel_token: Arc<AtomicBool>,                     // Walk-forward cancellation (global, shared across jobs)
    pub keep_monitor_in_background: Arc<AtomicBool>,          // Whether monitors survive window close
}
```

Additionally, `ChatSessionState` is managed as a separate Tauri state (`.manage(ChatSessionState::default())`), not inside AppState.

### Why each field exists:

- **client** uses `RwLock` (not `Mutex`) because OANDA credentials change after vault unlock, and multiple commands need concurrent read access
- **credential_vault** wraps `Option` because the vault starts locked (None) and gets populated after the user enters their master password
- **wf_cancel_token** is a single global `AtomicBool` - this is known technical debt (BUG-028) because it means only one walk-forward job can run at a time
- **keep_monitor_in_background** defaults to `false`, meaning monitors stop when the watcher window closes unless the user explicitly enables background mode

## Window Management Architecture

### Window Types and Labels

The app uses a multi-window architecture. Each window type has a fixed label (singleton) or timestamp-based label (multi-instance):

| Label | Type | Singleton? | Default Size (macOS) |
|-------|------|-----------|---------------------|
| `login` | Login/auth | Yes | 480x640 |
| `account` | Account dashboard | Yes | 1200x800 |
| `watcher` | Live Monitor | Yes | 900x700 |
| `backtest` | Strategy/Research | Yes | 1236x800 |
| `tradeanalysis` | Trade Analysis | Yes | 1200x900 |
| `ticket-{timestamp}` | Trading ticket | No (multi) | 315x610 |
| `chart-{timestamp}` | Chart | No (multi) | 1400x800 |

### Window Creation Pattern

All windows use the same pattern:
1. Check if singleton window exists -> focus it if so
2. Build URL via `create_webview_url()` which returns `tauri://` in dev, `http://localhost:14201/` in production
3. Apply `platform_size()` to add Windows padding (+20w, +30h)
4. Build via `WebviewWindowBuilder` with common settings: zoom hotkeys enabled, DISABLE_ZOOM_SCRIPT initialization, dark background color `(0x0e, 0x11, 0x17)`
5. Call `disable_magnification()` on macOS

### Frontend Window Detection

`src/index.tsx` reads `?window=` from the URL query string to determine which React component to render. Each window type lazy-loads only its own component tree:

- `login` or no param -> `DesktopLoginApp`
- `account` -> `App` (account dashboard)
- `backtest` -> `BacktestApp`
- `chart` -> `ChartApp`
- `ticket` -> `TradingTicketApp`
- `watcher` -> `StrategyWatcherApp`
- `tradeanalysis` -> `TradeAnalysisApp`

The login window has its own auth flow (`LoginWindowApp`). All other windows use `AppWindowWrapper` which requires authentication, provides Zero context, and auto-closes on sign-out.

## Menu Bar Construction (macOS)

Built in the setup closure using Tauri's menu API:

- **CandleSight** - About, Services, Hide/Show, Quit (all PredefinedMenuItems)
- **File** - Close Window
- **Edit** - Undo, Redo, Cut, Copy, Paste, Select All (required for text input to work on macOS)
- **View** - Account (Cmd+1), Live Monitor (Cmd+2), Research (Cmd+B), Trade Analysis (Cmd+T), Check for Updates
- **Window** - Minimize, Maximize, Fullscreen

Custom menu events spawn async tasks via `tauri::async_runtime::spawn` to open/focus the corresponding window. The "Check for Updates" menu item emits a `check-for-updates` event to the frontend.

**Known issue (BUG-061):** On Windows, the menu bar only appears on the first window opened. Once closed, no other windows have a menu bar.

## System Tray Integration

Defined in `tray.rs`. Always enabled for all users (not behind a feature gate).

### Tray Menu Items:
- **Show CandleSight** - Finds and focuses an existing window (priority: watcher > account > any non-login). If no windows exist, creates a new account window.
- **Stop All Monitors** - Directly stops all watcher handles by setting stop signals. Works even with no windows open.
- **Quit** - Calls `app.exit(0)` for immediate termination.

### Tray Behavior:
- Uses template icon (`tray-iconTemplate.png`) on macOS for auto-coloring
- Falls back to default window icon if template not found
- Left click shows menu (via `show_menu_on_left_click(true)`)
- macOS `ExitRequested` is prevented so app stays in tray when all windows close

## Command Registration Pattern

All commands are registered in a single `tauri::generate_handler![]` macro invocation in `main.rs`. This is a flat list of ~100+ function names. The pattern:

1. Command functions are defined in `src-tauri/src/commands/*.rs` modules (or inline in `main.rs` for simple ones)
2. Each module exports its commands via `pub` functions with `#[tauri::command]` attribute
3. `main.rs` imports commands with explicit `use` statements at the top
4. All commands listed in `invoke_handler(tauri::generate_handler![...])` - order does not matter but they are grouped by domain with comments

Adding a command requires: define the function, export it, import it in `main.rs`, add to the handler list.

## Config Loading with Compile-Time Fallbacks

`Config::load_with_compile_time()` implements a two-tier priority system:

1. **Runtime** - `dotenvy` loads `.env` from multiple locations:
   - Current directory (dev mode)
   - `src-tauri/.env` (when running from project root)
   - `~/Library/Application Support/com.candlesight.app/.env` (installed app)
   - Legacy path `com.fx-tracker.app/.env` for backwards compatibility
2. **Compile-time** - `option_env!()` values baked into the binary during CI builds

Priority: runtime > compile-time for each variable. Exception: `ANTHROPIC_API_KEY` is never embedded at compile time (security decision).

### Build Mode:
- `Dev` - Pro tier uses Haiku, Advanced uses Sonnet
- `Staging` / `Prod` - Pro uses Sonnet, Advanced uses Opus

## Deep Link Handling (OAuth Callbacks)

The app registers `candlesight://` as a custom URL scheme. When the browser completes OAuth, it redirects to `candlesight://callback?code=...&state=...`.

### macOS:
- `tauri_plugin_deep_link` handles the URL, parses code/state, emits `oauth-callback` event to frontend

### Windows:
- `tauri_plugin_single_instance` catches the second instance launch (since Windows launches a new exe for the deep link)
- Parses `candlesight://callback?code=...&state=...` from command line args
- Emits `oauth-callback` event and focuses the existing window
- Handles both `callback?` and `callback/?` URL patterns

### Dev mode:
- Deep links do not work in `tauri dev`, so a local HTTP server (`dev_server`) listens for OAuth callbacks and forwards them via a channel

## Localhost Plugin (Critical for Production)

`tauri-plugin-localhost` serves the frontend on `http://localhost:14201` in production builds. This is required because:

1. Clerk authentication requires an HTTP origin (not `tauri://`)
2. Without it, Clerk SDK initialization fails silently
3. The `capabilities/default.json` must grant remote API access to `http://localhost:*` for events to work from localhost-served pages

**If you remove the localhost plugin, authentication breaks in production builds.**

## Invariants

1. **AppState is immutable after construction** - Individual fields are interior-mutable (Arc<Mutex>, AtomicBool) but the struct itself does not change
2. **Login window is the only window defined in tauri.conf.json** - All other windows are created programmatically
3. **All windows use the same dark background color** - `Color(0x0e, 0x11, 0x17, 0xff)` to prevent white flash during load
4. **DISABLE_ZOOM_SCRIPT must be injected in every window** - Both via `initialization_script` (Rust side) and event listeners (frontend side, index.tsx)
5. **disable_magnification() must be called after every WebviewWindowBuilder.build()** on macOS
6. **OAuth config missing = app crash** - The `require_env` calls use `.expect()`, intentionally panicking if OAuth vars are not set
7. **Singleton windows focus-if-exists** - Before creating, always check `app_handle.get_webview_window(label)` and focus if found
8. **Chart and ticket windows are multi-instance** - Labels include timestamps: `chart-{ms}`, `ticket-{ms}`

## Windows-Specific Issues

### BUG-058 (CRITICAL): Backtest crashes on Windows
Running a backtest freezes the window and then crashes the app. Blocks all backtesting on Windows. Root cause not yet identified.

### BUG-061 (CRITICAL): Menu bar lost on Windows
Menu bar only appears on the first window opened. Once that window is closed, no other windows have a menu bar. Only fix is full app restart (which itself requires Task Manager due to BUG-055).

### BUG-055: App does not close properly on Windows
Fixed by adding `#[cfg(target_os = "macos")]` to the `ExitRequested` handler that prevents exit. Awaiting build verification.

### BUG-063: Windows sometimes stuck loading
Intermittent issue where opening a window gets stuck on the loading state.

## Known Technical Debt

1. **Global walk-forward cancel token** - `wf_cancel_token` is shared across all jobs. Should be per-job to support concurrent walk-forwards (BUG-028).
2. **Duplicate window creation code** - `open_startup_windows()` in `window.rs` and `open_*_window_internal()` in `main.rs` duplicate window creation logic. The internal functions exist because menu event handlers need them, but the code is nearly identical.
3. **Platform-specific code scattered** - `#[cfg(target_os = ...)]` blocks appear in main.rs, window.rs, tray.rs, and notifications.rs without a centralized platform abstraction.
4. **No Windows notification support** - `notifications.rs` has macOS-only implementations with no-op fallbacks for other platforms.
5. **Window type aliases** - `open_startup_windows` supports aliases like "trading" -> "account" and "backtesting" -> "backtest" with TODO to remove after migration.
6. **Menu bar analysis window size mismatch** - `open_analysis_window_internal` in main.rs uses 1400x900, but `open_analysis_window` in window.rs and `open_startup_windows` both use 1200x900.
