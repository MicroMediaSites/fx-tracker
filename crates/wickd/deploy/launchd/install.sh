#!/usr/bin/env bash
#
# Install a wickd LaunchAgent (macOS) — AGT-629.
#
# Job kinds:
#
#   install.sh stream [INSTRUMENTS] [--env ENV] [--account NAME] [--wickd PATH] [--dry-run]
#       Supervise the `wickd stream` socket hub — one OANDA subscription fanned
#       out over ~/.wickd/stream.sock to every watcher. Singleton: label
#       com.openthink.wickd-stream.
#
#   install.sh watch STRATEGY INSTRUMENTS [--account NAME] [--granularity G] \
#                    [--units N] [--slug SLUG] [--wickd PATH] [--dry-run]
#       Supervise ONE autonomous `wickd watch <strategy> <instruments> --auto`
#       (practice only). Parameterized per strategy: label
#       com.openthink.wickd-watch.<slug> (slug defaults to <strategy>-<account>),
#       so many strategies coexist as independent jobs.
#
#   install.sh calendar [--interval SECS] [--wickd PATH] [--dry-run]
#       Periodic one-shot `wickd calendar sync` merging the ForexFactory
#       weekly feed into ~/.wickd/calendar/. Singleton: label
#       com.openthink.wickd-calendar. Default interval 21600s (6h). No
#       OANDA credentials involved.
#
#   install.sh feed [--interval SECS] [--model M] [--claude PATH] \
#                   [--claude-config-dir DIR] [--wickd PATH] [--dry-run]
#       Periodic one-shot `wickd feed tick` producing the AI market-awareness
#       feed (~/.wickd/feed.ndjson) via one headless `claude -p` run per tick
#       on the logged-in Claude subscription. Singleton: label
#       com.openthink.wickd-feed. Default interval 900s (15m). Weekend ticks
#       and "nothing new" runs are quiet no-ops. PATH in the plist gains the
#       claude binary's directory; CLAUDE_CONFIG_DIR selects the account.
#
#   install.sh books [INSTRUMENTS] [--interval SECS] [--env ENV] \
#                    [--account NAME] [--wickd PATH] [--dry-run]
#       Periodic one-shot `wickd books <instruments> --store` collecting
#       order/position-book snapshots into ~/.wickd/books.db. Singleton:
#       label com.openthink.wickd-books. Default interval 1200s (OANDA's
#       20-minute snapshot cadence); the store is idempotent, so the interval
#       only affects fetch traffic.
#
#   install.sh watchdog [--interval SECS] [--grace SECS] [--realert SECS] [--dry-run]
#       Periodic one-shot candle watchdog: checks every installed
#       com.openthink.wickd-watch.* job for bar closes that went unprocessed
#       (and for the job vanishing from launchd) and posts a macOS
#       notification when a watcher has gone blind. Singleton: label
#       com.openthink.wickd-watchdog. Defaults: --interval 300, --grace 1200,
#       --realert 3600. Copies wickd-candle-watchdog.py to
#       ~/Library/Application Support/wickd-watchdog/.
#
# INSTRUMENTS is a single comma-separated token, e.g. "EUR_USD,GBP_USD" (clap
# splits it) or "all". Common options:
#   --wickd PATH   absolute path to the wickd binary (default: resolve from PATH)
#   --dry-run      render + validate the plist and print where it WOULD install,
#                  but do not copy or load it (used by the AGT-629 smoke test)
#
# The rendered plist is validated with `plutil -lint` before install. Loading
# uses the modern `launchctl bootstrap` (falling back to legacy `load -w`).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LA_DIR="${HOME}/Library/LaunchAgents"
LOG_DIR="${HOME}/Library/Logs/wickd"

die() { echo "error: $*" >&2; exit 1; }

usage() {
    sed -n '3,58p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit "${1:-1}"
}

# --- Slugify: lowercase, non-alnum -> '-', collapse/trim dashes. Keeps the
#     Label and log filenames filesystem- and launchd-safe. ----------------------
slugify() {
    printf '%s' "$1" \
        | tr '[:upper:]' '[:lower:]' \
        | sed -e 's/[^a-z0-9]\{1,\}/-/g' -e 's/^-//' -e 's/-$//'
}

# --- Resolve + normalise the wickd binary to an absolute path -----------------
resolve_wickd() {
    local bin="${1:-}"
    if [[ -z "${bin}" ]]; then
        bin="$(command -v wickd)" \
            || die "could not find 'wickd' on PATH; pass --wickd /abs/path/to/wickd"
    fi
    [[ -x "${bin}" ]] || die "'${bin}' is not an executable file"
    printf '%s/%s' "$(cd "$(dirname "${bin}")" && pwd)" "$(basename "${bin}")"
}

# --- Render a template with sed placeholder substitution, validate, and either
#     install+load it or (dry-run) just report. Args: template dest label dry k=v... -
render_and_install() {
    local template="$1" dest="$2" label="$3" dry="$4"
    shift 4
    local -a subs=()
    local pair
    for pair in "$@"; do
        subs+=(-e "s|${pair%%=*}|${pair#*=}|g")
    done

    local tmp
    tmp="$(mktemp)"
    sed "${subs[@]}" "${template}" >"${tmp}"

    if ! plutil -lint "${tmp}" >/dev/null; then
        rm -f "${tmp}"
        die "rendered plist failed validation (plutil -lint)"
    fi

    if [[ "${dry}" == "1" ]]; then
        echo "dry-run: validated ${label}"
        echo "  would install : ${dest}"
        echo "  rendered plist:"
        sed 's/^/    /' "${tmp}"
        rm -f "${tmp}"
        return 0
    fi

    mkdir -p "${LA_DIR}" "${LOG_DIR}"
    cp "${tmp}" "${dest}"
    rm -f "${tmp}"
    chmod 600 "${dest}"  # user-only on principle (contains no secret)
    echo "installed ${dest}"

    local domain="gui/$(id -u)"
    if launchctl bootstrap "${domain}" "${dest}" 2>/dev/null; then
        echo "bootstrapped ${label} into ${domain}"
    else
        launchctl bootout "${domain}/${label}" 2>/dev/null || true
        if launchctl bootstrap "${domain}" "${dest}" 2>/dev/null; then
            echo "re-bootstrapped ${label} into ${domain}"
        else
            launchctl load -w "${dest}"
            echo "loaded ${label} via legacy launchctl load"
        fi
    fi
    echo "verify: launchctl list | grep wickd"
}

# --- stream sub-command -------------------------------------------------------
install_stream() {
    local instruments="EUR_USD,GBP_USD,USD_JPY" env="practice" account="default"
    local wickd="" dry="0"
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --env)     env="$2"; shift 2 ;;
            --account) account="$2"; shift 2 ;;
            --wickd)   wickd="$2"; shift 2 ;;
            --dry-run) dry="1"; shift ;;
            -h|--help) usage 0 ;;
            --*)       die "unknown option: $1" ;;
            *)         instruments="$1"; shift ;;
        esac
    done

    local bin label dest template
    bin="$(resolve_wickd "${wickd}")"
    label="com.openthink.wickd-stream"
    dest="${LA_DIR}/${label}.plist"
    template="${SCRIPT_DIR}/${label}.plist"

    echo "stream hub: ${bin} stream ${instruments} --env ${env} --account ${account}"
    echo "  logs: ${LOG_DIR}/stream.{out,err}.log"
    render_and_install "${template}" "${dest}" "${label}" "${dry}" \
        "__WICKD_BIN__=${bin}" \
        "__HOME__=${HOME}" \
        "__LOG_DIR__=${LOG_DIR}" \
        "__INSTRUMENTS__=${instruments}" \
        "__ENV__=${env}" \
        "__ACCOUNT__=${account}"
}

# --- watch sub-command --------------------------------------------------------
install_watch() {
    [[ $# -ge 2 ]] || die "watch needs STRATEGY and INSTRUMENTS (see --help)"
    local strategy="$1" instruments="$2"; shift 2
    local account="default" granularity="H1" units="1000" slug="" wickd="" dry="0"
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --account)     account="$2"; shift 2 ;;
            --granularity) granularity="$2"; shift 2 ;;
            --units)       units="$2"; shift 2 ;;
            --slug)        slug="$2"; shift 2 ;;
            --wickd)       wickd="$2"; shift 2 ;;
            --dry-run)     dry="1"; shift ;;
            -h|--help)     usage 0 ;;
            *)             die "unknown option: $1" ;;
        esac
    done

    [[ -n "${slug}" ]] || slug="${strategy}-${account}"
    slug="$(slugify "${slug}")"
    [[ -n "${slug}" ]] || die "could not derive a slug from strategy/account"

    local bin label dest template
    bin="$(resolve_wickd "${wickd}")"
    label="com.openthink.wickd-watch.${slug}"
    dest="${LA_DIR}/${label}.plist"
    template="${SCRIPT_DIR}/com.openthink.wickd-watch.plist"

    echo "watcher [${slug}]: ${bin} watch ${strategy} ${instruments} \\"
    echo "    --granularity ${granularity} --env practice --account ${account} --units ${units} --auto"
    echo "  logs: ${LOG_DIR}/watch.${slug}.{out,err}.log"
    render_and_install "${template}" "${dest}" "${label}" "${dry}" \
        "__WICKD_BIN__=${bin}" \
        "__HOME__=${HOME}" \
        "__LOG_DIR__=${LOG_DIR}" \
        "__LABEL__=${label}" \
        "__SLUG__=${slug}" \
        "__STRATEGY__=${strategy}" \
        "__INSTRUMENTS__=${instruments}" \
        "__GRANULARITY__=${granularity}" \
        "__ACCOUNT__=${account}" \
        "__UNITS__=${units}"
}

# --- books sub-command --------------------------------------------------------
install_books() {
    local instruments="EUR_USD,GBP_USD,USD_JPY,USD_CHF,AUD_USD,USD_CAD,NZD_USD,EUR_GBP"
    local interval="1200" env="practice" account="default" wickd="" dry="0"
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --interval) interval="$2"; shift 2 ;;
            --env)      env="$2"; shift 2 ;;
            --account)  account="$2"; shift 2 ;;
            --wickd)    wickd="$2"; shift 2 ;;
            --dry-run)  dry="1"; shift ;;
            -h|--help)  usage 0 ;;
            --*)        die "unknown option: $1" ;;
            *)          instruments="$1"; shift ;;
        esac
    done
    [[ "${interval}" =~ ^[0-9]+$ && "${interval}" -gt 0 ]] \
        || die "--interval must be a positive integer (seconds)"

    local bin label dest template
    bin="$(resolve_wickd "${wickd}")"
    label="com.openthink.wickd-books"
    dest="${LA_DIR}/${label}.plist"
    template="${SCRIPT_DIR}/${label}.plist"

    echo "books collector: ${bin} books ${instruments} --store --env ${env} --account ${account}"
    echo "  every ${interval}s; logs: ${LOG_DIR}/books.{out,err}.log; store: ~/.wickd/books.db"
    render_and_install "${template}" "${dest}" "${label}" "${dry}" \
        "__WICKD_BIN__=${bin}" \
        "__HOME__=${HOME}" \
        "__LOG_DIR__=${LOG_DIR}" \
        "__INSTRUMENTS__=${instruments}" \
        "__INTERVAL__=${interval}" \
        "__ENV__=${env}" \
        "__ACCOUNT__=${account}"
}

# --- calendar sub-command -------------------------------------------------------
install_calendar() {
    local interval="21600" wickd="" dry="0"
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --interval) interval="$2"; shift 2 ;;
            --wickd)    wickd="$2"; shift 2 ;;
            --dry-run)  dry="1"; shift ;;
            -h|--help)  usage 0 ;;
            --*)        die "unknown option: $1" ;;
            *)          die "unexpected argument: $1" ;;
        esac
    done
    [[ "${interval}" =~ ^[0-9]+$ && "${interval}" -gt 0 ]] \
        || die "--interval must be a positive integer (seconds)"

    local bin label dest template
    bin="$(resolve_wickd "${wickd}")"
    label="com.openthink.wickd-calendar"
    dest="${LA_DIR}/${label}.plist"
    template="${SCRIPT_DIR}/${label}.plist"

    echo "calendar sync: ${bin} calendar sync"
    echo "  every ${interval}s; logs: ${LOG_DIR}/calendar.{out,err}.log; store: ~/.wickd/calendar/"
    render_and_install "${template}" "${dest}" "${label}" "${dry}" \
        "__WICKD_BIN__=${bin}" \
        "__HOME__=${HOME}" \
        "__LOG_DIR__=${LOG_DIR}" \
        "__INTERVAL__=${interval}"
}

# --- feed sub-command -----------------------------------------------------------
install_feed() {
    local interval="900" model="" wickd="" claude="" claude_config_dir="" dry="0"
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --interval)          interval="$2"; shift 2 ;;
            --model)             model="$2"; shift 2 ;;
            --wickd)             wickd="$2"; shift 2 ;;
            --claude)            claude="$2"; shift 2 ;;
            --claude-config-dir) claude_config_dir="$2"; shift 2 ;;
            --dry-run)           dry="1"; shift ;;
            -h|--help)           usage 0 ;;
            --*)                 die "unknown option: $1" ;;
            *)                   die "unexpected argument: $1" ;;
        esac
    done
    [[ "${interval}" =~ ^[0-9]+$ && "${interval}" -gt 0 ]] \
        || die "--interval must be a positive integer (seconds)"

    # Resolve the claude binary so the plist PATH can carry its directory
    # (launchd's default PATH misses ~/.local/bin, where claude usually lives).
    if [[ -z "${claude}" ]]; then
        claude="$(command -v claude)" \
            || die "could not find 'claude' on PATH; pass --claude /abs/path/to/claude"
    fi
    [[ -x "${claude}" ]] || die "'${claude}' is not an executable file"
    local claude_dir
    claude_dir="$(cd "$(dirname "${claude}")" && pwd)"

    # Which Claude Code account the tick bills to. Defaults to the caller's
    # CLAUDE_CONFIG_DIR, then the stock ~/.claude.
    if [[ -z "${claude_config_dir}" ]]; then
        claude_config_dir="${CLAUDE_CONFIG_DIR:-${HOME}/.claude}"
    fi
    [[ -d "${claude_config_dir}" ]] \
        || die "claude config dir '${claude_config_dir}' does not exist (pass --claude-config-dir)"

    local bin label dest template
    bin="$(resolve_wickd "${wickd}")"
    label="com.openthink.wickd-feed"
    dest="${LA_DIR}/${label}.plist"
    template="${SCRIPT_DIR}/${label}.plist"

    echo "feed producer: ${bin} feed tick --model ${model:-sonnet}"
    echo "  every ${interval}s; claude: ${claude} (config: ${claude_config_dir})"
    echo "  logs: ${LOG_DIR}/feed.{out,err}.log; store: ~/.wickd/feed.ndjson"
    render_and_install "${template}" "${dest}" "${label}" "${dry}" \
        "__WICKD_BIN__=${bin}" \
        "__HOME__=${HOME}" \
        "__LOG_DIR__=${LOG_DIR}" \
        "__INTERVAL__=${interval}" \
        "__MODEL__=${model:-sonnet}" \
        "__CLAUDE_DIR__=${claude_dir}" \
        "__CLAUDE_CONFIG_DIR__=${claude_config_dir}"
}

# --- watchdog sub-command -------------------------------------------------------
install_watchdog() {
    local interval="300" grace="1200" realert="3600" dry="0"
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --interval) interval="$2"; shift 2 ;;
            --grace)    grace="$2"; shift 2 ;;
            --realert)  realert="$2"; shift 2 ;;
            --dry-run)  dry="1"; shift ;;
            -h|--help)  usage 0 ;;
            *)          die "unknown option: $1" ;;
        esac
    done
    local n
    for n in interval grace realert; do
        [[ "${!n}" =~ ^[0-9]+$ && "${!n}" -gt 0 ]] \
            || die "--${n} must be a positive integer (seconds)"
    done

    local label dest template script_src script_dest
    label="com.openthink.wickd-watchdog"
    dest="${LA_DIR}/${label}.plist"
    template="${SCRIPT_DIR}/${label}.plist"
    script_src="${SCRIPT_DIR}/wickd-candle-watchdog.py"
    script_dest="${HOME}/Library/Application Support/wickd-watchdog/wickd-candle-watchdog.py"
    [[ -f "${script_src}" ]] || die "missing ${script_src}"

    echo "candle watchdog: python3 ${script_dest} --grace ${grace} --realert ${realert}"
    echo "  every ${interval}s; logs: ${LOG_DIR}/watchdog.{out,err}.log; alerts via macOS notifications"
    if [[ "${dry}" != "1" ]]; then
        mkdir -p "$(dirname "${script_dest}")"
        cp "${script_src}" "${script_dest}"
        chmod 755 "${script_dest}"
        echo "installed ${script_dest}"
    fi
    render_and_install "${template}" "${dest}" "${label}" "${dry}" \
        "__SCRIPT__=${script_dest}" \
        "__HOME__=${HOME}" \
        "__LOG_DIR__=${LOG_DIR}" \
        "__INTERVAL__=${interval}" \
        "__GRACE__=${grace}" \
        "__REALERT__=${realert}"
}

# --- dispatch -----------------------------------------------------------------
[[ $# -ge 1 ]] || usage 1
case "$1" in
    stream)   shift; install_stream "$@" ;;
    watch)    shift; install_watch "$@" ;;
    books)    shift; install_books "$@" ;;
    calendar) shift; install_calendar "$@" ;;
    feed)    shift; install_feed "$@" ;;
    watchdog) shift; install_watchdog "$@" ;;
    -h|--help) usage 0 ;;
    *)      die "unknown job kind '$1' (expected 'stream', 'watch', 'books', 'calendar', 'feed', or 'watchdog')" ;;
esac
