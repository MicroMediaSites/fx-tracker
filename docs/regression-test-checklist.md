# CandleSight Regression Test Checklist

> **Run before each production release on both macOS and Windows.**

## Pre-Release Checklist

- [X] All automated tests pass (`npm run test:be`, `npm run test:ui`) ✓ macOS 2026-01-25
- [ ] Build succeeds on both platforms (CI green)
- [ ] Version number updated in `tauri.conf.json`

---

## Installation & Launch

| Test | macOS | Windows |
|------|-------|---------|
| Fresh install from DMG/MSI | [ ] | [ ] |
| App launches without crash | [ ] | [ ] |
| App icon displays correctly (dock/taskbar) | [ ] | [ ] |
| Menu bar/system tray icon works | [ ] | [ ] |

---

## Authentication

| Test | macOS | Windows |
|------|-------|---------|
| Sign up flow (new user) | [ ] | [ ] |
| Sign in flow (existing user) | [ ] | [ ] |
| OAuth redirect returns to app | [ ] | [ ] |
| Sign out clears session | [ ] | [ ] |
| Token refresh works (stay signed in) | [ ] | [ ] |

---

## OANDA Integration

| Test | macOS | Windows |
|------|-------|---------|
| Add OANDA credentials (demo account) | [ ] | [ ] |
| Add OANDA credentials (live account) | [ ] | [ ] |
| Switch between demo/live accounts | [ ] | [ ] |
| Account balance displays correctly | [ ] | [ ] |
| Open positions sync | [ ] | [ ] |
| Trade history syncs | [ ] | [ ] |

---

## Streaming & Real-Time

| Test | macOS | Windows |
|------|-------|---------|
| Price stream connects | [ ] | [ ] |
| Prices update in real-time | [ ] | [ ] |
| Stream reconnects after disconnect | [ ] | [ ] |
| Stream health indicator accurate | [ ] | [ ] |

---

## Charting

| Test | macOS | Windows |
|------|-------|---------|
| Chart loads with candles | [ ] | [ ] |
| Timeframe switching works | [ ] | [ ] |
| Instrument switching works | [ ] | [ ] |
| Chart indicators render | [ ] | [ ] |
| Trade markers display (Premium+) | [ ] | [ ] |
| S/R zones display (Premium+) | [ ] | [ ] |
| Zoom/pan works smoothly | [ ] | [ ] |

---

## Strategy Builder

| Test | macOS | Windows |
|------|-------|---------|
| Create new strategy | [ ] | [ ] |
| Add indicators | [ ] | [ ] |
| Add entry rules | [ ] | [ ] |
| Add exit rules | [ ] | [ ] |
| Configure risk settings | [ ] | [ ] |
| Save strategy | [ ] | [ ] |
| Edit existing strategy | [ ] | [ ] |
| Delete strategy (with confirmation) | [ ] | [ ] |
| JSON import (Pro) | [ ] | [ ] |
| JSON export | [ ] | [ ] |
| AI strategy builder (Pro) | [ ] | [ ] |

---

## Backtesting

| Test | macOS | Windows |
|------|-------|---------|
| Run single backtest | [ ] | [ ] |
| Results display correctly | [ ] | [ ] |
| Equity curve renders | [ ] | [ ] |
| Trade list populates | [ ] | [ ] |
| Run optimization (grid search) | [ ] | [ ] |
| Run walk-forward analysis | [ ] | [ ] |
| Apply best parameters | [ ] | [ ] |
| AI analysis (Pro) | [ ] | [ ] |

---

## Live Monitor (Strategy Watcher)

| Test | macOS | Windows |
|------|-------|---------|
| Create new monitor | [ ] | [ ] |
| Monitor detects pattern matches | [ ] | [ ] |
| Desktop notification fires | [ ] | [ ] |
| Notification click opens chart | [ ] | [ ] |
| Stop/start monitor | [ ] | [ ] |
| Delete monitor | [ ] | [ ] |

---

## Trading

| Test | macOS | Windows |
|------|-------|---------|
| Open FX Ticket window | [ ] | [ ] |
| Place market order | [ ] | [ ] |
| Set stop loss | [ ] | [ ] |
| Set take profit | [ ] | [ ] |
| Order executes successfully | [ ] | [ ] |
| Position appears in list | [ ] | [ ] |

---

## Notes & Journal

| Test | macOS | Windows |
|------|-------|---------|
| Add note to trade | [ ] | [ ] |
| Add note to strategy | [ ] | [ ] |
| Delete note | [ ] | [ ] |
| "Add to Notes" from AI analysis (Pro) | [ ] | [ ] |

---

## Settings & Preferences

| Test | macOS | Windows |
|------|-------|---------|
| Open settings modal | [ ] | [ ] |
| Change data source (demo/live) | [ ] | [ ] |
| Configure startup windows | [ ] | [ ] |
| Settings persist after restart | [ ] | [ ] |

---

## Window Management

| Test | macOS | Windows |
|------|-------|---------|
| Open multiple windows (View menu) | [ ] | [ ] |
| Keyboard shortcuts work (Cmd/Ctrl+B, etc.) | [ ] | [ ] |
| Windows restore position on restart | [ ] | [ ] |
| Close window (X button) | [ ] | [ ] |
| Quit app fully exits | [ ] | [ ] |

---

## Auto-Updater

| Test | macOS | Windows |
|------|-------|---------|
| Update notification appears | [ ] | [ ] |
| Download progress shows | [ ] | [ ] |
| Update installs successfully | [ ] | [ ] |
| App restarts after update | [ ] | [ ] |

---

## Edge Cases & Error Handling

| Test | macOS | Windows |
|------|-------|---------|
| Offline mode (no internet) | [ ] | [ ] |
| Invalid OANDA credentials | [ ] | [ ] |
| Expired session recovery | [ ] | [ ] |

---

## Sign-Off

| Role | Name | Date | Platform |
|------|------|------|----------|
| Developer | Matt | 2026-01-25 | macOS |
| Developer | | | Windows |
| Beta Tester | | | |

---

## Notes

_Record any issues found during testing:_

```
[Date] [Platform] [Issue Description]
```
