#!/usr/bin/env bash
#
# Unit tests for scripts/verify-auto-update.sh (AGT-644).
#
# Stubs `gh` and `curl` on PATH so the release/feed/endpoint phases run
# against local fixtures — no network, no GitHub auth, no app launch
# (every case runs --feed-only or fails before the app phase).
#
# Run: scripts/verify-auto-update.test.sh

set -euo pipefail

HARNESS="$(cd "$(dirname "$0")" && pwd)/verify-auto-update.sh"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

STUBS="$WORK/stubs"
FIXTURES="$WORK/fixtures"
mkdir -p "$STUBS" "$FIXTURES"

# --- stub gh -----------------------------------------------------------
cat > "$STUBS/gh" <<'EOF'
#!/usr/bin/env bash
case "$1" in
  api)
    cat "$FIXTURES/release.json"
    ;;
  release)
    # gh release download ... --output <path> ...
    out=""
    prev=""
    for a in "$@"; do
      [ "$prev" = "--output" ] && out="$a"
      prev="$a"
    done
    cp "$FIXTURES/latest.json" "$out"
    ;;
  *)
    echo "stub gh: unexpected args: $*" >&2
    exit 1
    ;;
esac
EOF

# --- stub curl ---------------------------------------------------------
cat > "$STUBS/curl" <<'EOF'
#!/usr/bin/env bash
[ -f "$FIXTURES/endpoint.json" ] || { echo "stub curl: endpoint down" >&2; exit 22; }
cat "$FIXTURES/endpoint.json"
EOF

chmod +x "$STUBS/gh" "$STUBS/curl"
export FIXTURES
export PATH="$STUBS:$PATH"

# --- fixtures ----------------------------------------------------------
write_release() { # tag
  printf '{"tag_name": "%s", "draft": false, "prerelease": false}\n' "$1" > "$FIXTURES/release.json"
}
write_feed() { # version [omit-signature]
  if [ "${2:-}" = "omit-signature" ]; then
    printf '{"version": "%s", "platforms": {"darwin-aarch64": {"url": "https://example.com/wickd_aarch64.app.tar.gz"}}}\n' "$1"
  else
    printf '{"version": "%s", "platforms": {"darwin-aarch64": {"url": "https://example.com/wickd_aarch64.app.tar.gz", "signature": "sig"}}}\n' "$1"
  fi > "$FIXTURES/latest.json"
}
write_endpoint() { # version
  printf '{"version": "%s"}\n' "$1" > "$FIXTURES/endpoint.json"
}

# --- assertions --------------------------------------------------------
PASS=0
FAIL=0

run_case() { # name expected-exit out-dir args...
  local name="$1" expected_exit="$2" out="$3"
  shift 3
  local actual_exit=0
  "$HARNESS" --out "$out" "$@" > "$out.log" 2>&1 || actual_exit=$?
  if [ "$actual_exit" = "$expected_exit" ]; then
    echo "ok   - $name (exit $actual_exit)"
    PASS=$((PASS + 1))
  else
    echo "FAIL - $name: expected exit $expected_exit, got $actual_exit"
    sed 's/^/       /' "$out.log"
    FAIL=$((FAIL + 1))
  fi
}

assert_json() { # name file jq-expr
  local name="$1" file="$2" expr="$3"
  if jq -e "$expr" "$file" >/dev/null 2>&1; then
    echo "ok   - $name"
    PASS=$((PASS + 1))
  else
    echo "FAIL - $name: $expr not satisfied in $file"
    jq . "$file" 2>/dev/null | sed 's/^/       /' || true
    FAIL=$((FAIL + 1))
  fi
}

# --- case 1: feed-only happy path (endpoint unresolvable -> skipped) ----
write_release v0.21.0
write_feed 0.21.0
rm -f "$FIXTURES/endpoint.json"
OUT1="$WORK/case1"
run_case "feed-only happy path passes" 0 "$OUT1" --feed-only --app "$WORK/no-such.app"
assert_json "case1 result pass=true" "$OUT1/result.json" '.pass == true'
assert_json "case1 expected_version" "$OUT1/result.json" '.expected_version == "0.21.0"'
assert_json "case1 release+feed pass" "$OUT1/result.json" \
  '[.phases[] | select(.name == "release" or .name == "feed") | .status] == ["pass", "pass"]'
assert_json "case1 endpoint skipped" "$OUT1/result.json" \
  '.phases[] | select(.name == "endpoint") | .status == "skipped"'
assert_json "case1 app skipped" "$OUT1/result.json" \
  '.phases[] | select(.name == "app") | .status == "skipped"'

# --- case 2: feed version mismatch fails --------------------------------
write_release v0.21.0
write_feed 0.20.9
OUT2="$WORK/case2"
run_case "stale latest.json fails" 1 "$OUT2" --feed-only --app "$WORK/no-such.app"
assert_json "case2 result pass=false" "$OUT2/result.json" '.pass == false'
assert_json "case2 feed phase failed" "$OUT2/result.json" \
  '.phases[] | select(.name == "feed") | .status == "fail"'

# --- case 3: missing darwin-aarch64 signature fails ----------------------
write_release v0.21.0
write_feed 0.21.0 omit-signature
OUT3="$WORK/case3"
run_case "missing darwin-aarch64 signature fails" 1 "$OUT3" --feed-only --app "$WORK/no-such.app"
assert_json "case3 feed phase failed" "$OUT3/result.json" \
  '.phases[] | select(.name == "feed") | .status == "fail"'

# --- case 4: explicit endpoint serving the released version passes -------
write_release v0.21.0
write_feed 0.21.0
write_endpoint 0.21.0
OUT4="$WORK/case4"
run_case "matching endpoint passes" 0 "$OUT4" --feed-only --endpoint https://example.com/releases/latest.json
assert_json "case4 endpoint pass" "$OUT4/result.json" \
  '.phases[] | select(.name == "endpoint") | .status == "pass"'

# --- case 5: endpoint serving a stale version fails -----------------------
write_release v0.21.0
write_feed 0.21.0
write_endpoint 0.20.0
OUT5="$WORK/case5"
run_case "stale endpoint fails" 1 "$OUT5" --feed-only --endpoint https://example.com/releases/latest.json
assert_json "case5 endpoint phase failed" "$OUT5/result.json" \
  '.phases[] | select(.name == "endpoint") | .status == "fail"'

# --- case 6: machine-readable trailer line -------------------------------
if grep -q '^VERIFY_UPDATE_RESULT: PASS$' "$OUT1.log" \
   && grep -q '^VERIFY_UPDATE_RESULT: FAIL$' "$OUT2.log"; then
  echo "ok   - trailer line present in pass and fail output"
  PASS=$((PASS + 1))
else
  echo "FAIL - trailer line missing from harness output"
  FAIL=$((FAIL + 1))
fi

echo ""
echo "verify-auto-update tests: $PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]
