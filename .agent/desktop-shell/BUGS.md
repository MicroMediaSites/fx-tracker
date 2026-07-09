# Desktop Shell Bugs

## Active Bugs

### BUG-058 | CRITICAL | Windows backtest crash
- **Status:** Open
- **Reported:** 2026-01-27
- **Symptom:** Running a backtest on Windows freezes the window and then crashes the app
- **Impact:** Blocks all backtesting functionality on Windows
- **Root cause:** Unknown. Needs investigation. Possibly related to heavy computation on the main thread or WebView2 interaction issues.
- **Notes:** Works fine on macOS. The crash may be in the backtest domain rather than desktop-shell, but the window freeze suggests a rendering/IPC issue.

### BUG-061 | CRITICAL | Menu bar lost on Windows
- **Status:** Open
- **Reported:** 2026-01-27
- **Symptom:** Menu bar only appears on the first window opened on Windows. Once that window is closed, no other windows have the menu bar and it cannot be recovered even by reopening windows.
- **Impact:** User loses access to View menu, preferences, and all menu actions. Only fix is full app restart (which itself requires Task Manager due to BUG-055).
- **Root cause:** Likely a Tauri 2 / Windows WebView2 issue where the menu is bound to the first window and not transferred when that window is destroyed.

### BUG-055 | Pending Verification | App does not close on Windows
- **Status:** Pending
- **Reported:** 2026-01-27
- **Symptom:** App does not close properly on Windows - requires Task Manager to kill the process
- **Root cause:** `RunEvent::ExitRequested` handler called `api.prevent_exit()` unconditionally on all platforms
- **Fix applied:** Added `#[cfg(target_os = "macos")]` to only prevent exit on macOS. Awaiting Windows build verification.

### BUG-063 | Open | Windows window stuck loading
- **Status:** Open
- **Reported:** 2026-01-29
- **Symptom:** Sometimes when opening a window on Windows, it gets stuck on the loading state
- **Impact:** Intermittent, does not always occur. May require window close and reopen.

### BUG-060 | Open | Keyboard shortcuts not working on Windows
- **Status:** Open
- **Reported:** 2026-01-27
- **Symptom:** Ctrl+B (backtest), Ctrl+Shift+C (chart), etc. do nothing on Windows. Only Ctrl+K (AI terminal) works.
- **Notes:** May be related to Tauri accelerator registration on Windows, or the shortcuts may be defined only in the macOS menu bar (which is lost per BUG-061).

### BUG-053 | Fixed | Chart window not opening on startup
- **Status:** Fixed
- **Fixed:** 2026-03-01
- **Symptom:** Charting window does not open on startup when configured in startup windows settings
- **Root cause:** Same issue as BUG-054 -- the startup window opening effect in `App.tsx` checked `windowLabel !== 'main'` but the account window label is `'account'`. This caused the entire `openStartupWindows()` function to return early without opening any configured windows. The primary login flow via `DesktopLoginApp.tsx` correctly uses the Rust `open_startup_windows` command, but the `App.tsx` fallback path was dead code.
- **Fix:** Changed `windowLabel !== 'main'` to `windowLabel !== 'account'` in `App.tsx`.
- **Prevention:** When referencing window labels in frontend code, always verify the label matches what `WebviewWindowBuilder::new()` uses in the Rust backend. See BUG-054 for the identical lesson.

### BUG-049 | Fixed | Test notification button silently no-ops
- **Status:** Fixed
- **Fixed:** 2026-03-01
- **Symptom:** Clicking "Send Test" in Settings > Desktop Notifications did nothing. No notification appeared, no error shown, no visual feedback.
- **Root cause:** The click handler had `if (!desktopNotifications) return;` which silently aborted when the notifications toggle was in the "Disabled" position (the default). Since the button had no disabled styling, the user could click it and see zero response. Additionally, even on success the handler gave no visual feedback confirming the notification was sent.
- **Fix:** Removed the `desktopNotifications` guard so the test notification always fires regardless of the enabled/disabled toggle (its purpose is to verify the OS notification system works). Added `isSendingTestNotification` loading state with "Sending..." text and disabled button during the invoke. Added `notificationSuccess` state that shows "Test notification sent. Check your notification center." for 3 seconds after success. Improved error message to suggest checking System Settings.
- **Prevention:** When adding "test" or "preview" buttons for features that have an enable/disable toggle, the test button should work regardless of the toggle state. The guard belongs on the production code path (real notifications), not the test path. Always provide visual feedback for user-initiated actions, especially when the result is external (OS notification center).

## Fixed Bugs (Reference)

### BUG-054 | Fixed | No notification shown when app update is available
- **Fixed:** 2026-03-01
- **Symptom:** Update downloads and installs correctly, but user is never shown a notification or UI indicator that an update exists. They don't know an update exists until the app restarts.
- **Root cause:** The startup update check in `App.tsx` guarded against running on non-main windows with `if (currentWindow.label !== 'main') return;`. However, the account window's label is `'account'` (set in `window.rs` via `WebviewWindowBuilder::new(&app_handle, "account", url)`), not `'main'`. The label `'main'` does not correspond to any window in the application. This meant the condition always returned early and the `check()` call from `@tauri-apps/plugin-updater` was never executed on startup.
- **Fix:** Changed the window label check from `'main'` to `'account'` in the startup update check effect in `App.tsx`. The manual "Check for Updates" menu path (via `check-for-updates` event) was unaffected because it does not have this guard.
- **Prevention:** When referencing window labels in frontend code, always verify the label matches what `WebviewWindowBuilder::new()` uses in the Rust backend. The window label table in ARCHITECTURE.md lists all labels. Never use hardcoded string labels without cross-referencing the actual window creation code.

### BUG-040 | Fixed | Pinch-to-zoom hijacked app zoom
- **Fixed:** 2026-01-28
- **Fix:** Triple-layer defense: (1) `initialization_script` with JS gesture/wheel listeners, (2) `disable_magnification()` via Objective-C on macOS, (3) Event listeners in `index.tsx`. All three are needed.
- **Regression risk:** Adding any new window without the DISABLE_ZOOM_SCRIPT and disable_magnification() call.

### BUG-019 | Fixed | Notification crash on bulk-clear
- **Fixed:** 2025-12-19
- **Root cause:** `mac-notification-sys` `wait_for_click(true)` blocks in FFI code. macOS bulk-clearing notifications causes segfault.
- **Fix:** Removed blocking wait. Also never use `main_button()` (BUG-020 - causes 100% CPU).

### BUG-029 | Fixed | Desktop reload loop and AI terminal wipe
- **Fixed:** 2026-01-19
- **Root cause (Part 1):** Desktop hooks importing from `ZeroContext` (web) instead of `DesktopZeroContext` (desktop)
- **Root cause (Part 2):** `LoginWindowApp` in `index.tsx` lacked `hasEverLoaded` ref, causing Zero provider remount during token refresh
- **Fix:** Changed context imports + added stability refs to prevent unmounting during brief auth state changes
