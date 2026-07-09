#!/usr/bin/env bash
#
# verify-auto-update.sh — agent-runnable auto-update verification harness (AGT-644)
#
# Proves the shipped app updates itself. Phases:
#   1. release  — resolve the latest published GitHub release (authenticated gh;
#                 the repo is PRIVATE, anonymous fetches 404).
#   2. feed     — download latest.json from that release and assert it
#                 advertises exactly the released version with a darwin-aarch64
#                 url + signature (the platform the macOS updater requires).
#   3. endpoint — GET the live update endpoint the installed app will actually
#                 hit and assert it serves the same version. The endpoint is
#                 deliberately NOT hardcoded: AGT-649/650 are retiring the
#                 Railway queries-service proxy, so it resolves from
#                 --endpoint, then $WICKD_UPDATE_ENDPOINT, then best-effort
#                 extraction from the installed binary (the URL baked in at
#                 build time — exactly what the updater will request).
#   4. app      — launch the installed prod app, drive the updater UI
#                 (tray "Check for Updates..." -> "Download & Install" ->
#                 "Restart Now"), and assert the installed + running version
#                 equals the latest release. Skipped with --feed-only.
#
# Machine-readable output an agent can gate on:
#   - exit code: 0 pass, 1 fail, 2 usage/environment error
#   - <out>/result.json with per-phase status
#   - final line "VERIFY_UPDATE_RESULT: PASS|FAIL"
# Review evidence: <out>/verify-update-<version>.png screenshot (app phase).
#
# Environment (app phase only): the invoking terminal/agent needs macOS
# Accessibility permission (osascript UI driving) and Screen Recording
# permission (screencapture). See docs/auto-update-verification.md.
#
# WARNING: the app phase launches and — when an update lands — RESTARTS the
# installed app. Do not run it while a live trading/watcher session depends on
# the running app.

set -euo pipefail

REPO="MicroMediaSites/fx-tracker"
APP_PATH="/Applications/wickd.app"
OUT="verify-update-artifacts"
TIMEOUT=300
FEED_ONLY=0
ENDPOINT="${WICKD_UPDATE_ENDPOINT:-}"

usage() {
  cat <<'EOF'
Usage: scripts/verify-auto-update.sh [options]

Options:
  --feed-only       Verify release + feed (+ endpoint if resolvable) only;
                    skip launching the installed app. For CI / machines that
                    do not have the prod app installed.
  --app PATH        Installed app bundle (default: /Applications/wickd.app)
  --endpoint URL    Update endpoint to verify (default: $WICKD_UPDATE_ENDPOINT,
                    else extracted from the installed binary, else skipped)
  --out DIR         Artifacts directory (default: verify-update-artifacts)
  --timeout SECS    Max seconds for the download/install/relaunch cycle
                    (default: 300)
  --repo OWNER/REPO GitHub repo (default: MicroMediaSites/fx-tracker)
  -h, --help        Show this help
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --feed-only) FEED_ONLY=1 ;;
    --app) APP_PATH="$2"; shift ;;
    --endpoint) ENDPOINT="$2"; shift ;;
    --out) OUT="$2"; shift ;;
    --timeout) TIMEOUT="$2"; shift ;;
    --repo) REPO="$2"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

for tool in gh jq curl; do
  command -v "$tool" >/dev/null 2>&1 || { echo "ERROR: required tool '$tool' not found" >&2; exit 2; }
done

mkdir -p "$OUT"

# ---------------------------------------------------------------------------
# Result accounting
# ---------------------------------------------------------------------------
PHASES_JSON="[]"
EXPECTED_VERSION=""
RELEASE_TAG=""
APP_UPDATED="false"
SCREENSHOT=""

record_phase() { # name status detail
  PHASES_JSON=$(jq -n --argjson acc "$PHASES_JSON" \
    --arg name "$1" --arg status "$2" --arg detail "$3" \
    '$acc + [{name: $name, status: $status, detail: $detail}]')
  echo "[$1] $2 — $3"
}

finish() { # pass(true|false)
  local pass="$1"
  jq -n \
    --argjson pass "$pass" \
    --arg repo "$REPO" \
    --arg tag "$RELEASE_TAG" \
    --arg expected "$EXPECTED_VERSION" \
    --argjson feed_only "$([ "$FEED_ONLY" = 1 ] && echo true || echo false)" \
    --argjson updated "$APP_UPDATED" \
    --arg screenshot "$SCREENSHOT" \
    --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --argjson phases "$PHASES_JSON" \
    '{harness: "verify-auto-update", timestamp: $ts, pass: $pass,
      repo: $repo, release_tag: $tag, expected_version: $expected,
      feed_only: $feed_only, app_updated: $updated,
      screenshot: (if $screenshot == "" then null else $screenshot end),
      phases: $phases}' > "$OUT/result.json"
  echo ""
  cat "$OUT/result.json"
  if [ "$pass" = "true" ]; then
    echo "VERIFY_UPDATE_RESULT: PASS"
    exit 0
  else
    echo "VERIFY_UPDATE_RESULT: FAIL"
    exit 1
  fi
}

fail_phase() { # name detail
  record_phase "$1" "fail" "$2"
  finish false
}

# ---------------------------------------------------------------------------
# Phase 1: release — latest published GitHub release (authenticated)
# ---------------------------------------------------------------------------
RELEASE_JSON=$(gh api "repos/$REPO/releases/latest" 2>&1) \
  || fail_phase "release" "cannot resolve latest release: $RELEASE_JSON"
RELEASE_TAG=$(jq -r '.tag_name // empty' <<<"$RELEASE_JSON")
[ -n "$RELEASE_TAG" ] || fail_phase "release" "latest release has no tag_name"
EXPECTED_VERSION="${RELEASE_TAG#v}"
record_phase "release" "pass" "latest published release is $RELEASE_TAG (version $EXPECTED_VERSION)"

# ---------------------------------------------------------------------------
# Phase 2: feed — latest.json on the release
# ---------------------------------------------------------------------------
FEED_FILE="$OUT/latest.json"
if ! DL_ERR=$(gh release download "$RELEASE_TAG" --repo "$REPO" \
    --pattern latest.json --output "$FEED_FILE" --clobber 2>&1); then
  fail_phase "feed" "cannot download latest.json from $RELEASE_TAG: $DL_ERR"
fi
FEED_VERSION=$(jq -r '.version // empty' "$FEED_FILE")
[ "$FEED_VERSION" = "$EXPECTED_VERSION" ] \
  || fail_phase "feed" "latest.json version '$FEED_VERSION' != released version '$EXPECTED_VERSION'"
jq -e '.platforms["darwin-aarch64"].url and .platforms["darwin-aarch64"].signature' \
    "$FEED_FILE" >/dev/null 2>&1 \
  || fail_phase "feed" "latest.json missing darwin-aarch64 url/signature"
record_phase "feed" "pass" "latest.json advertises $FEED_VERSION with darwin-aarch64 url+signature"

# ---------------------------------------------------------------------------
# Phase 3: endpoint — the live feed the installed app will hit
# ---------------------------------------------------------------------------
app_executable() { # bundle-path -> executable name (empty on failure)
  defaults read "$1/Contents/Info" CFBundleExecutable 2>/dev/null || true
}

ENDPOINT_SOURCE="flag/env"
if [ -z "$ENDPOINT" ] && [ -d "$APP_PATH" ]; then
  EXEC_NAME=$(app_executable "$APP_PATH")
  BIN="$APP_PATH/Contents/MacOS/$EXEC_NAME"
  if [ -n "$EXEC_NAME" ] && [ -f "$BIN" ]; then
    ENDPOINT=$(strings "$BIN" 2>/dev/null | grep -oE 'https://[^" ]*latest\.json' | head -1 || true)
    ENDPOINT_SOURCE="extracted from installed binary"
  fi
fi

if [ -n "$ENDPOINT" ]; then
  if ! EP_BODY=$(curl -fsSL --max-time 30 "$ENDPOINT" 2>&1); then
    fail_phase "endpoint" "GET $ENDPOINT failed: $EP_BODY"
  fi
  EP_VERSION=$(jq -r '.version // empty' <<<"$EP_BODY" 2>/dev/null || true)
  [ "$EP_VERSION" = "$EXPECTED_VERSION" ] \
    || fail_phase "endpoint" "endpoint $ENDPOINT serves version '$EP_VERSION' != released '$EXPECTED_VERSION'"
  record_phase "endpoint" "pass" "$ENDPOINT ($ENDPOINT_SOURCE) serves $EP_VERSION"
else
  record_phase "endpoint" "skipped" "no endpoint resolvable (pass --endpoint or set WICKD_UPDATE_ENDPOINT)"
fi

# ---------------------------------------------------------------------------
# Phase 4: app — launch installed app, drive updater, assert version
# ---------------------------------------------------------------------------
if [ "$FEED_ONLY" = 1 ]; then
  record_phase "app" "skipped" "--feed-only requested"
  finish true
fi

[ -d "$APP_PATH" ] || fail_phase "app" "installed app bundle not found at $APP_PATH"
EXEC_NAME=$(app_executable "$APP_PATH")
[ -n "$EXEC_NAME" ] || fail_phase "app" "cannot read CFBundleExecutable from $APP_PATH"

installed_version() {
  defaults read "$APP_PATH/Contents/Info" CFBundleShortVersionString 2>/dev/null || true
}
app_running() {
  pgrep -f "$APP_PATH/Contents/MacOS/" >/dev/null 2>&1
}

# osascript helpers. Both need Accessibility permission for the invoking
# terminal/agent. The updater modal is a WKWebView; its HTML buttons are
# exposed to System Events as accessibility buttons named by their text.
click_button() { # button-name -> echoes clicked|not-found
  osascript - "$EXEC_NAME" "$1" 2>/dev/null <<'APPLESCRIPT' || echo "not-found"
on run argv
  set procName to item 1 of argv
  set btnName to item 2 of argv
  tell application "System Events"
    tell process procName
      set frontmost to true
      repeat with w in windows
        try
          repeat with el in entire contents of w
            try
              if class of el is button then
                if (name of el is btnName) or (description of el is btnName) then
                  click el
                  return "clicked"
                end if
              end if
            end try
          end repeat
        end try
      end repeat
    end tell
  end tell
  return "not-found"
end run
APPLESCRIPT
}

trigger_tray_update_check() { # -> echoes clicked|not-found
  # Tray menu item defined in src-tauri/src/main.rs: "Check for Updates..."
  osascript - "$EXEC_NAME" 2>/dev/null <<'APPLESCRIPT' || echo "not-found"
on run argv
  set procName to item 1 of argv
  tell application "System Events"
    tell process procName
      try
        click menu bar item 1 of menu bar 2
        delay 0.5
        click menu item "Check for Updates..." of menu 1 of menu bar item 1 of menu bar 2
        return "clicked"
      end try
    end tell
  end tell
  return "not-found"
end run
APPLESCRIPT
}

take_screenshot() {
  SCREENSHOT="$OUT/verify-update-$EXPECTED_VERSION.png"
  if ! screencapture -x "$SCREENSHOT" 2>/dev/null; then
    SCREENSHOT=""
    echo "WARNING: screencapture failed (Screen Recording permission missing?)" >&2
  fi
}

VERSION_BEFORE=$(installed_version)
[ -n "$VERSION_BEFORE" ] || fail_phase "app" "cannot read installed version from $APP_PATH"
echo "[app] installed version before: $VERSION_BEFORE (expected: $EXPECTED_VERSION)"
echo "[app] WARNING: this launches (and on update, restarts) $APP_PATH"

open -a "$APP_PATH" || fail_phase "app" "failed to launch $APP_PATH"
LAUNCH_DEADLINE=$(( $(date +%s) + 30 ))
until app_running; do
  [ "$(date +%s)" -lt "$LAUNCH_DEADLINE" ] || fail_phase "app" "app did not start within 30s"
  sleep 1
done
echo "[app] app is running"

if [ "$VERSION_BEFORE" = "$EXPECTED_VERSION" ]; then
  # Nothing to update: the assertion (running version == latest release)
  # already holds. Recorded as updated=false so agents can tell the two
  # pass modes apart.
  take_screenshot
  record_phase "app" "pass" "installed app already at latest version $EXPECTED_VERSION (no update exercised)"
  finish true
fi

# Drive the updater UI. The startup silent check (App.tsx) opens the update
# modal ~2s after launch in the 'account' window; the tray trigger is the
# deterministic fallback.
echo "[app] waiting for update modal ('Download & Install')..."
CLICKED_DOWNLOAD=0
MODAL_DEADLINE=$(( $(date +%s) + 90 ))
TRAY_TRIED=0
sleep 5
while [ "$(date +%s)" -lt "$MODAL_DEADLINE" ]; do
  if [ "$(click_button "Download & Install")" = "clicked" ]; then
    CLICKED_DOWNLOAD=1
    break
  fi
  if [ "$TRAY_TRIED" = 0 ]; then
    echo "[app] modal not found yet; triggering tray 'Check for Updates...'"
    trigger_tray_update_check >/dev/null
    TRAY_TRIED=1
  fi
  sleep 3
done
if [ "$CLICKED_DOWNLOAD" != 1 ]; then
  take_screenshot
  fail_phase "app" "update modal 'Download & Install' button never appeared (Accessibility permission granted? screenshot: ${SCREENSHOT:-none})"
fi
echo "[app] clicked 'Download & Install'; waiting for download to finish..."

CLICKED_RESTART=0
RESTART_DEADLINE=$(( $(date +%s) + TIMEOUT ))
while [ "$(date +%s)" -lt "$RESTART_DEADLINE" ]; do
  if [ "$(click_button "Restart Now")" = "clicked" ]; then
    CLICKED_RESTART=1
    break
  fi
  sleep 5
done
if [ "$CLICKED_RESTART" != 1 ]; then
  take_screenshot
  fail_phase "app" "'Restart Now' never appeared within ${TIMEOUT}s (download stalled? screenshot: ${SCREENSHOT:-none})"
fi
echo "[app] clicked 'Restart Now'; waiting for relaunch at $EXPECTED_VERSION..."

VERIFY_DEADLINE=$(( $(date +%s) + 120 ))
while [ "$(date +%s)" -lt "$VERIFY_DEADLINE" ]; do
  VERSION_AFTER=$(installed_version)
  if [ "$VERSION_AFTER" = "$EXPECTED_VERSION" ] && app_running; then
    APP_UPDATED="true"
    sleep 3 # let the relaunched UI render before evidence capture
    open -a "$APP_PATH" 2>/dev/null || true
    take_screenshot
    record_phase "app" "pass" "app updated $VERSION_BEFORE -> $VERSION_AFTER and is running (screenshot: ${SCREENSHOT:-none})"
    finish true
  fi
  sleep 3
done

take_screenshot
fail_phase "app" "app did not come back at $EXPECTED_VERSION within 120s (installed: $(installed_version), running: $(app_running && echo yes || echo no), screenshot: ${SCREENSHOT:-none})"
