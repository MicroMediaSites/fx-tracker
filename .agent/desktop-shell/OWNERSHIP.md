# Desktop Shell Domain

## Description

The desktop-shell domain owns the Tauri application shell: the main entry point, application state, window management, menu bar, system tray, app lifecycle, configuration loading, deep link handling, and desktop notifications. This domain is the glue that holds the application together. It does not own any feature logic (trading, backtesting, etc.) but it provides the infrastructure those features depend on.

**Fragility warning:** This domain is especially fragile on Windows, where multiple critical bugs (BUG-058, BUG-061) remain open. Changes must be tested on both macOS and Windows.

## Owned Files

### Rust backend (src-tauri/src/)
- `src-tauri/src/main.rs` - App entry point, AppState struct, command registration, menu bar, lifecycle events, window management commands defined inline
- `src-tauri/src/config.rs` - Config struct, CompileTimeConfig, BuildMode, OandaEnvironment, .env loading
- `src-tauri/src/error.rs` - Error enum (thiserror-based), Result type alias
- `src-tauri/src/lib.rs` - Module declarations and public re-exports for the `candlesight_lib` crate
- `src-tauri/src/tray.rs` - System tray setup, menu items, show/stop/quit actions
- `src-tauri/src/notifications.rs` - macOS native notifications via mac-notification-sys, global enable/disable flag
- `src-tauri/src/commands/window.rs` - Window creation for all types (backtest, chart, analysis, ticket, watcher, login), LOCALHOST_PORT, create_webview_url, disable_magnification, DISABLE_ZOOM_SCRIPT
- `src-tauri/src/commands/mod.rs` - Command module declarations (shared; other domains add their modules here)

### Configuration
- `src-tauri/tauri.conf.json` - Tauri window config, CSP, bundle settings, updater, deep-link scheme
- `src-tauri/capabilities/default.json` - Permission grants including remote API access for localhost

### Frontend entry points
- `src/index.tsx` - React bootstrap, window type detection via URL params, lazy loading, ErrorBoundary, auth gating
- `src/App.tsx` - Account window root component (main window after login)

## Shared File: commands/mod.rs

`src-tauri/src/commands/mod.rs` re-exports all command modules. When any domain adds a new command module, it must add a `pub mod` line here. This file is touched by multiple domains but owned by desktop-shell for coordination purposes.

## Glob Patterns

```
src-tauri/src/main.rs
src-tauri/src/config.rs
src-tauri/src/error.rs
src-tauri/src/lib.rs
src-tauri/src/tray.rs
src-tauri/src/notifications.rs
src-tauri/src/commands/window.rs
src-tauri/src/commands/mod.rs
src-tauri/tauri.conf.json
src-tauri/capabilities/**/*
src/index.tsx
src/App.tsx
```

## Primary Stack

- **Backend:** Rust + Tauri 2 (tauri v2 with WebviewWindow API)
- **Frontend entry points:** React 19 + TypeScript

## Key Dependencies

### Tauri Plugins (registered in main.rs builder chain)
- `tauri-plugin-shell` - Opening external URLs
- `tauri-plugin-notification` - Notification permissions
- `tauri-plugin-window-state` - Persist window position/size (POSITION + SIZE flags only, not maximized state)
- `tauri-plugin-localhost` - **Critical for production** - Serves frontend on `http://localhost:14201` so Clerk auth works (needs HTTP origin, not tauri://)
- `tauri-plugin-updater` - Auto-update support
- `tauri-plugin-process` - App restart after update
- `tauri-plugin-deep-link` - OAuth callback handling via `candlesight://` scheme
- `tauri-plugin-single-instance` - **Windows only** - Routes deep links from second instance to existing app

### Rust Crates
- `mac-notification-sys` - macOS native notifications (conditional compilation)
- `objc` - macOS WebView magnification control (BUG-040 fix)
- `dotenvy` - .env file loading
- `urlencoding` - URL parameter encoding for window URLs
- `tracing` / `tracing-subscriber` - Structured logging
- `thiserror` - Error derive macros
- `tokio` - Async runtime (main function is `#[tokio::main]`)

### Frontend
- `@tauri-apps/api/core` - invoke() for Tauri commands
- `@tauri-apps/api/event` - listen() for event streams
- `@tauri-apps/api/webviewWindow` - getCurrentWebviewWindow()
- `@tauri-apps/plugin-updater` - check() for auto-updates
- `wouter` - Client-side routing
