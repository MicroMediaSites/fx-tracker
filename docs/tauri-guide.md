# Tauri Guide

Patterns and gotchas for working with Tauri 2 in this project.

## Localhost Plugin (Production/Staging Builds)

In production and staging builds, we use `tauri-plugin-localhost` to serve the frontend from `http://localhost:14201` instead of the default `tauri://` protocol. (Historical: this was originally required for Clerk authentication, which needed an HTTP origin. Clerk is gone; the plugin stays because window URLs, CSP, and the capabilities config are built around the localhost origin.)

### Remote API Access Required

When using the localhost plugin, Tauri's IPC (including events) won't work unless you explicitly grant remote API access in `capabilities/default.json`:

```json
{
  "windows": ["*"],
  "remote": {
    "urls": ["http://localhost:*", "http://127.0.0.1:*"]
  },
  "permissions": [...]
}
```

**Symptoms if missing:** Events emit successfully from Rust (`app_handle.emit()` returns `Ok()`), but the frontend never receives them. Invoke commands still work fine.

**References:**
- [Tauri Capabilities - Remote API Access](https://v2.tauri.app/security/capabilities/)
- [GitHub Issue #1974](https://github.com/tauri-apps/plugins-workspace/issues/1974)

## Event Listeners

### Set Up Listeners Before Starting Streams

When setting up event listeners for streaming data, always register the listener BEFORE invoking the command that starts the stream:

```typescript
// CORRECT: Listener first, then start stream
useEffect(() => {
  let unlisten: UnlistenFn | null = null;

  const start = async () => {
    // 1. Set up listener first
    unlisten = await listen<PriceUpdate>('price-update', (event) => {
      handlePrice(event.payload);
    });

    // 2. Then start the stream
    await invoke('start_price_stream', { instruments });
  };

  start();
  return () => unlisten?.();
}, [instruments]);
```

```typescript
// WRONG: Race condition - may miss early events
useEffect(() => {
  const start = async () => {
    await invoke('start_price_stream', { instruments }); // Stream starts
    // Events may be emitted here before listener is ready!
    unlisten = await listen<PriceUpdate>('price-update', ...);
  };
  // ...
}, []);
```

### Shared Streams Across Windows

Multiple windows can listen to the same event stream. The `start_price_stream` command is idempotent - if the stream is already running, it returns success without starting a duplicate.

```rust
// In streaming.rs - idempotent start
if self.running.compare_exchange(false, true, ...).is_err() {
    return Ok(()); // Already running, no-op
}
```

Windows should NOT stop the stream on cleanup since other windows may be using it:

```typescript
return () => {
  unlisten?.();
  // Don't call stop_price_stream - other windows may need it
};
```

## Window Management

### Window Labels

Each window type has a specific label used for targeting:
- `login` - Login window (defined in tauri.conf.json)
- `trading` - Main trading window
- `watcher` - Strategy watcher
- `ticket` - Trading ticket
- `chart` - Chart window
- `backtest` - Backtest window

### Creating Windows

Windows are created dynamically using `WebviewWindowBuilder`:

```rust
WebviewWindowBuilder::new(&app_handle, "chart", url)
    .title("CandleSight - Chart")
    .inner_size(1200.0, 800.0)
    .build()?;
```

In production, URLs use the localhost plugin:
```rust
#[cfg(not(debug_assertions))]
let url = format!("http://localhost:{}/{}", LOCALHOST_PORT, path);
```

## Debugging Tips

### Events Not Reaching Frontend

1. Check `capabilities/default.json` has the `remote` section for localhost
2. Verify the event name matches exactly (case-sensitive)
3. Check listener is set up before the emitting code runs
4. In dev, use browser console; in staging, add temporary UI debug elements

### Stream Not Starting

1. Check credentials are set (vault unlocked)
2. Verify `is_streaming()` returns expected value
3. Check for errors in the Rust logs
