# Desktop Shell Conventions

## How to Add a New Window Type

Adding a new window type requires changes in both Rust and TypeScript.

### Rust side (src-tauri/src/commands/window.rs):

1. Add a new `#[tauri::command]` function following the existing pattern:

```rust
#[tauri::command]
pub async fn open_my_window(app_handle: AppHandle) -> Result<(), String> {
    // 1. Check if singleton window already exists - focus it if so
    if let Some(window) = app_handle.get_webview_window("mywindow") {
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    // 2. Build URL using create_webview_url (handles dev vs production)
    let url = create_webview_url("index.html?window=mywindow");

    // 3. Apply platform_size for Windows padding
    let (w, h) = platform_size(1200.0, 800.0);

    // 4. Build with standard settings
    let window = WebviewWindowBuilder::new(&app_handle, "mywindow", url)
        .title("CandleSight - My Window")
        .inner_size(w, h)
        .resizable(true)
        .zoom_hotkeys_enabled(true)
        .initialization_script(DISABLE_ZOOM_SCRIPT)  // BUG-040: prevent zoom
        .background_color(Color(0x0e, 0x11, 0x17, 0xff))  // Dark bg to prevent flash
        .build()
        .map_err(|e| e.to_string())?;

    // 5. Disable magnification on macOS
    disable_magnification(&window);

    Ok(())
}
```

2. If the window should be available via the startup windows feature, add a case in `open_startup_windows()` in window.rs.

3. If the window should be available via the macOS menu bar, add a `MenuItem` and handler in the setup closure in main.rs, plus an `open_my_window_internal()` function.

### Registration (src-tauri/src/main.rs):

1. Import the command: `use commands::window::open_my_window;`
2. Add to the `invoke_handler(tauri::generate_handler![..., open_my_window])` list

### Frontend side (src/index.tsx):

1. Create a new lazy-loaded app component:
```typescript
const MyWindowApp = lazy(() => import('./MyWindowApp').then((m) => ({ default: m.MyWindowApp })));
```

2. Add a case in the `getMainApp()` switch inside `AppWindowWrapper`:
```typescript
case 'mywindow':
    return MyWindowApp;
```

### For multi-instance windows (like charts):

Use a timestamp-based label instead of a fixed string:
```rust
let timestamp = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_millis();
let window_label = format!("mywindow-{}", timestamp);
```

## How to Add a New Tauri Command

1. **Define the function** in the appropriate `src-tauri/src/commands/*.rs` module:

```rust
#[tauri::command]
pub async fn my_command(
    state: tauri::State<'_, AppState>,
    app_handle: AppHandle,
    some_param: String,
) -> Result<SomeReturnType, String> {
    // Implementation
    // Errors must be String (Tauri IPC limitation)
    // Use .map_err(|e| e.to_string())? for error conversion
}
```

2. **Export from the module** - Functions must be `pub`.

3. **Import in main.rs**:
```rust
use commands::mymodule::my_command;
```

4. **Register in the handler list** in main.rs:
```rust
.invoke_handler(tauri::generate_handler![
    // ... existing commands ...
    my_command,
])
```

5. **Call from frontend**:
```typescript
const result = await invoke<ReturnType>('my_command', { someParam: 'value' });
```

Note: Tauri automatically converts snake_case Rust parameter names to camelCase for the frontend.

## How to Add a New Tauri Plugin

1. Add the crate to `src-tauri/Cargo.toml`
2. Add `.plugin(tauri_plugin_name::init())` to the builder chain in main.rs (order generally does not matter)
3. Add required permissions to `src-tauri/capabilities/default.json`
4. If the plugin needs frontend access, add the npm package and import in the relevant component

## Error Handling Patterns

### In main.rs setup:
- **Missing OAuth config** - Uses `.expect()` to panic. This is intentional: the app cannot function without auth config.
- **Optional services** (Claude, spread stats) - Log and continue if initialization fails. These are degraded-mode acceptable.
- **Tray setup failure** - `eprintln!` and continue. Tray is nice-to-have.

### In command handlers:
- All command return types must be `Result<T, String>` because Tauri IPC does not support custom error types
- Convert errors with `.map_err(|e| e.to_string())?`
- The `Error` enum in `error.rs` uses `thiserror` but is only used within the library, not at the command boundary

### In lifecycle event handlers:
- Use `try_lock()` instead of `lock()` to avoid deadlocks in the synchronous `RunEvent` handler
- Failures are logged but never propagated (the handler cannot return errors)

## Configuration Patterns

### Adding a new config variable:

1. Add the field to `Config` struct in `config.rs`
2. Add a compile-time field to `CompileTimeConfig` if it should be embeddable
3. Load with runtime-then-compile-time priority:
```rust
let my_var = std::env::var("MY_VAR")
    .ok()
    .or_else(|| compile_time.my_var.map(String::from));
```
4. In main.rs, pass to `CompileTimeConfig`:
```rust
Config::load_with_compile_time(CompileTimeConfig {
    my_var: option_env!("MY_VAR"),
    ..
})
```

### Security rule:
Never embed secrets (API keys, database passwords) at compile time. Use runtime .env or route through the queries-service proxy.

## Platform-Conditional Code Patterns

### Compile-time platform checks:
```rust
#[cfg(target_os = "macos")]
{
    // macOS-specific code
}

#[cfg(target_os = "windows")]
{
    // Windows-specific code
}

#[cfg(not(target_os = "windows"))]
{
    // Everything except Windows
}
```

### Debug vs Release:
```rust
#[cfg(debug_assertions)]     // Dev builds
#[cfg(not(debug_assertions))] // Production/staging builds
```

### Combined platform + build mode:
```rust
#[cfg(any(target_os = "macos", target_os = "windows"))]
```

### Conditional plugin registration:
The single-instance plugin is Windows-only because macOS handles deep links natively:
```rust
#[cfg(target_os = "windows")]
let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
    // ...
}));
```

### In notifications.rs:
Provide a real implementation for macOS and a no-op fallback:
```rust
#[cfg(target_os = "macos")]
pub fn send_pattern_match_notification(...) { /* real impl */ }

#[cfg(not(target_os = "macos"))]
pub fn send_pattern_match_notification(...) { /* log and return */ }
```

## Anti-Patterns

### Never do these:

1. **Do not create windows without `disable_magnification()` and `DISABLE_ZOOM_SCRIPT`** - BUG-040 will resurface. Both the Objective-C runtime call and the JS injection are needed.

2. **Do not use `lock()` in `RunEvent` handlers** - These run on the main thread. Use `try_lock()` to avoid deadlocks. If the lock fails, skip the operation and log.

3. **Do not remove the localhost plugin** - Clerk authentication breaks in production without HTTP origin. This is not negotiable.

4. **Do not add `.env` values to `CompileTimeConfig` for secrets** - API keys must not be embedded in the binary. Only non-sensitive config (build mode, service URLs) should use compile-time fallbacks.

5. **Do not skip the singleton check for singleton windows** - Always check `get_webview_window(label)` before creating. Duplicate windows with the same label will panic.

6. **Do not use `api.prevent_exit()` on Windows** - BUG-055. Windows users expect closing all windows to quit the app. The `ExitRequested` handler must be `#[cfg(target_os = "macos")]` only.

7. **Do not call `Notification.main_button()` from mac-notification-sys** - BUG-020. It causes the send() function to run an NSRunLoop that busy-polls forever, consuming 100% CPU per notification.

8. **Do not assume the `claude` field in AppState is `Some`** - It is `None` when no API key is configured. Always check `if let Some(ref claude) = state.claude`.

9. **Do not start streams before setting up event listeners** - Race condition documented in tauri-guide.md. Listener must be registered before invoking the command that starts the stream.

10. **Do not navigate login window by closing and recreating it** - Use `window.navigate(url)` instead to avoid close/recreate race conditions (see `open_login_window` in window.rs).
