#!/usr/bin/env bash
#
# Uninstall a wickd LaunchAgent (macOS) — AGT-629.
#
#   uninstall.sh stream                    stop + remove the stream-hub job
#   uninstall.sh watch SLUG                stop + remove one watcher job
#   uninstall.sh books                     stop + remove the books collector
#   uninstall.sh calendar                  stop + remove the calendar sync
#   uninstall.sh feed                      stop + remove the feed producer
#   uninstall.sh watchdog                  stop + remove the candle watchdog
#   uninstall.sh --all [--purge-logs]      stop + remove EVERY wickd job
#
# Logs under ~/Library/Logs/wickd are left in place unless --purge-logs is
# given. Stopping uses the modern `launchctl bootout` (falling back to legacy
# `unload -w`).

set -euo pipefail

LA_DIR="${HOME}/Library/LaunchAgents"
LOG_DIR="${HOME}/Library/Logs/wickd"
DOMAIN="gui/$(id -u)"

die() { echo "error: $*" >&2; exit 1; }

usage() {
    sed -n '3,15p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit "${1:-1}"
}

# Stop (bootout/unload) and remove one label's plist.
remove_label() {
    local label="$1"
    local dest="${LA_DIR}/${label}.plist"
    if launchctl bootout "${DOMAIN}/${label}" 2>/dev/null; then
        echo "booted out ${DOMAIN}/${label}"
    elif [[ -f "${dest}" ]] && launchctl unload -w "${dest}" 2>/dev/null; then
        echo "unloaded ${label} via legacy launchctl unload"
    else
        echo "${label} was not loaded"
    fi
    if [[ -f "${dest}" ]]; then
        rm -f "${dest}"
        echo "removed ${dest}"
    else
        echo "no plist at ${dest}"
    fi
}

purge_logs_if_asked() {
    if [[ "${1:-}" == "--purge-logs" ]]; then
        rm -rf "${LOG_DIR}"
        echo "removed logs at ${LOG_DIR}"
    else
        echo "logs left in place at ${LOG_DIR} (use --purge-logs to remove)"
    fi
}

[[ $# -ge 1 ]] || usage 1
case "$1" in
    stream)
        remove_label "com.openthink.wickd-stream"
        purge_logs_if_asked "${2:-}"
        ;;
    watch)
        [[ $# -ge 2 ]] || die "watch needs a SLUG (see: launchctl list | grep wickd-watch)"
        remove_label "com.openthink.wickd-watch.$2"
        purge_logs_if_asked "${3:-}"
        ;;
    books)
        remove_label "com.openthink.wickd-books"
        purge_logs_if_asked "${2:-}"
        ;;
    calendar)
        remove_label "com.openthink.wickd-calendar"
        purge_logs_if_asked "${2:-}"
        ;;
    feed)
        remove_label "com.openthink.wickd-feed"
        purge_logs_if_asked "${2:-}"
        ;;
    watchdog)
        remove_label "com.openthink.wickd-watchdog"
        rm -rf "${HOME}/Library/Application Support/wickd-watchdog"
        echo "removed ${HOME}/Library/Application Support/wickd-watchdog"
        purge_logs_if_asked "${2:-}"
        ;;
    --all)
        # Enumerate loaded wickd jobs, plus any installed plists not currently loaded.
        labels="$(
            {
                launchctl list 2>/dev/null | awk '/com\.openthink\.wickd/ {print $3}'
                ls "${LA_DIR}" 2>/dev/null \
                    | sed -n 's/^\(com\.openthink\.wickd[^/]*\)\.plist$/\1/p'
            } | sort -u
        )"
        if [[ -z "${labels}" ]]; then
            echo "no wickd jobs installed"
        else
            while IFS= read -r label; do
                [[ -n "${label}" ]] && remove_label "${label}"
            done <<< "${labels}"
        fi
        purge_logs_if_asked "${2:-}"
        ;;
    -h|--help) usage 0 ;;
    *) die "unknown target '$1' (expected 'stream', 'watch SLUG', 'books', 'calendar', 'feed', 'watchdog', or '--all')" ;;
esac
