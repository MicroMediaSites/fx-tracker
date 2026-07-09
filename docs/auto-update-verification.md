# Auto-Update Verification (AGT-644)

How to prove the shipped app updates itself to the latest GitHub release.
Two layers exist; this doc covers both and tells a build agent exactly what
to run as a **post-release step**.

## Layers

| Layer | What | Where it runs | When |
|-------|------|---------------|------|
| CI feed check | `.github/workflows/verify-update.yml` — validates `latest.json` + release assets through the authenticated API (the repo is private) | `[self-hosted, Linux, ARM64]` runner | Automatically on every `release: published` |
| Full-loop harness | `scripts/verify-auto-update.sh` — resolves the latest release, validates the feed and the live endpoint, then launches the installed prod app, drives the updater UI, and asserts the running version equals the release | The Mac where the prod app is installed | Run by an agent (or Matt) after a release, as the final proof |

## Running the harness

```bash
# Full verification (launches /Applications/wickd.app and, if an update is
# available, updates + restarts it):
scripts/verify-auto-update.sh

# Feed + endpoint only (safe anywhere; no app launch). Use in CI or on
# machines without the installed app:
scripts/verify-auto-update.sh --feed-only
```

Options: `--app PATH` (default `/Applications/wickd.app`), `--endpoint URL`,
`--out DIR` (default `verify-update-artifacts/`, gitignored), `--timeout SECS`
(download/install budget, default 300), `--repo OWNER/REPO`.

## What an agent gates on

- **Exit code**: `0` pass, `1` fail, `2` usage/environment error.
- **`verify-update-artifacts/result.json`**: `.pass` boolean plus per-phase
  `{name, status: pass|fail|skipped, detail}` records for `release`, `feed`,
  `endpoint`, `app`. `app_updated` distinguishes a real update cycle (`true`)
  from an already-at-latest pass (`false`).
- **Trailer line**: the last stdout line is `VERIFY_UPDATE_RESULT: PASS|FAIL`.
- **Review evidence**: `verify-update-artifacts/verify-update-<version>.png`
  screenshot (captured on app-phase success *and* failure). Attach it to the
  ticket's review evidence.

Unit tests for the harness logic: `scripts/verify-auto-update.test.sh`
(stubs `gh`/`curl`; no network, no app launch).

## The endpoint seam (AGT-649/650)

The updater feed is whatever URL was baked into the installed binary at build
time (currently `${VITE_QUERIES_SERVICE_URL}/releases/latest.json` via the
Railway queries-service proxy — the repo is private, so the proxy injects a
server-side GitHub token). **AGT-649/650 are retiring that Railway path.**
The harness therefore never hardcodes an endpoint; it resolves one in order:

1. `--endpoint URL`
2. `$WICKD_UPDATE_ENDPOINT`
3. Best-effort extraction from the installed binary
   (`strings <app>/Contents/MacOS/<exe> | grep …latest.json`) — this is the
   URL the updater will *actually* request, so it stays correct across the
   migration automatically.
4. Otherwise the endpoint phase is `skipped` (not a failure).

After AGT-649/650 land, nothing in the harness needs to change: freshly built
apps carry the new URL and extraction keeps working. To pin CI or scripted
runs to the new feed explicitly, pass it via `--endpoint`/`WICKD_UPDATE_ENDPOINT`.

## Requirements

- `gh` authenticated with access to `MicroMediaSites/fx-tracker` (private
  repo — anonymous release fetches 404), plus `jq` and `curl`.
- **App phase only** (macOS TCC, one-time grants to the invoking
  terminal/agent):
  - *Accessibility* — the harness drives the update modal ("Download &
    Install" → "Restart Now") and the tray "Check for Updates..." item via
    `osascript`/System Events.
  - *Screen Recording* — `screencapture` for the evidence screenshot.

## Safety

The app phase launches — and, when an update lands, **restarts** — the
installed app. Do not run it while a live trading/watcher session depends on
the running app (see the `com.openthink.wickd-*` launchd jobs). `--feed-only`
never touches the app.

## Known limits (pre-runner reality, 2026-07-06)

- No self-hosted runners are registered yet, so no post-rebrand release
  exists; the harness has been exercised against the last real release
  (v0.20.22) in `--feed-only` mode, including live endpoint extraction from
  the installed binary. The UI-driving app phase (modal clicks, tray trigger,
  relaunch polling) is only provable once a real release run produces an
  update to install — see HUMAN-ITEMS.md.
- The update modal's buttons are reached through the WKWebView accessibility
  tree; if the modal copy changes ("Download & Install" / "Restart Now" in
  `src/components/ui/UpdateModal.tsx`, "Check for Updates..." in
  `src-tauri/src/main.rs`), update the button names in
  `scripts/verify-auto-update.sh` to match.
