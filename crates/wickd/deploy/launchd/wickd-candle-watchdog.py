#!/usr/bin/env python3
"""wickd candle watchdog — external liveness check for `wickd watch` jobs.

Runs as a periodic launchd one-shot (label com.openthink.wickd-watchdog, see
install.sh `watchdog`). It answers one question the watchers cannot answer
about themselves: *are candle closes actually being processed?* A watcher
attached to a wedged stream hub keeps running and logging heartbeats while
processing nothing (observed 2026-07-09..13, three days blind) — an external
check against wall-clock bar boundaries is the belt to the in-process
REST-fallback braces.

For every installed com.openthink.wickd-watch.*.plist it:

  1. reads the watcher's granularity + instruments from ProgramArguments,
  2. reads the newest `watcher-tick` per instrument from the watcher's
     out.log (ticks log the bar START time; close = start + granularity),
  3. computes the bar closes that have elapsed since then (dailyAlignment=2,
     UTC — H4 closes 02/06/10/14/18/22Z, same anchor the whole system uses),
  4. flags any close older than --grace that fell inside market-open hours,
  5. checks the launchd job is still loaded at all,

and raises a macOS notification (osascript) when anything is wrong. Weekend
closes are skipped via a conservative closed-market window (Fri 20:00Z →
Sun 22:30Z, covering both US DST phases plus reopen slack) so quiet weekends
never page. Re-alerts for one continuing stall are throttled to once per
--realert seconds; a NEW missed close always alerts immediately.

State (first-run baseline + last-alert times) lives in
~/Library/Application Support/wickd-watchdog/state.json — deliberately NOT
in ~/.wickd, which belongs to the daemons.

Exit code: 0 = all healthy (or nothing to check), 1 = at least one problem
found (launchd records it; the alert already went out either way).
"""

import argparse
import glob
import json
import os
import subprocess
import sys
from datetime import datetime, timedelta, timezone

LAUNCH_AGENTS = os.path.expanduser("~/Library/LaunchAgents")
LOG_DIR = os.path.expanduser("~/Library/Logs/wickd")
STATE_PATH = os.path.expanduser(
    "~/Library/Application Support/wickd-watchdog/state.json"
)
WATCH_PLIST_GLOB = "com.openthink.wickd-watch.*.plist"

# Bar-boundary anchor: dailyAlignment=2, alignmentTimezone=UTC (see CLAUDE.md
# "OANDA Candle Alignment"). Every H* granularity steps from 02:00 UTC.
ALIGNMENT_ANCHOR_HOUR = 2

GRANULARITY_MINUTES = {
    "M1": 1, "M2": 2, "M4": 4, "M5": 5, "M10": 10, "M15": 15, "M30": 30,
    "H1": 60, "H2": 120, "H3": 180, "H4": 240, "H6": 360, "H8": 480,
    "H12": 720, "D": 1440,
}

# Conservative closed-market window (UTC). OANDA FX closes Fri 21:00 or
# 22:00 UTC depending on US DST and reopens Sun 21:00/22:00; the padding
# swallows both phases and the thin reopen minutes. A bar close inside this
# window is never flagged — we trade a little Friday-night coverage for zero
# weekend false alarms.
CLOSED_FROM = (4, 20, 0)   # Friday 20:00 UTC  (weekday 4)
CLOSED_TO = (6, 22, 30)    # Sunday 22:30 UTC  (weekday 6)


def market_open_at(t: datetime) -> bool:
    """True when FX is (conservatively) considered open at UTC instant t."""
    wd, hm = t.weekday(), (t.hour, t.minute)
    if wd == 5:  # Saturday
        return False
    if wd == 4 and hm >= CLOSED_FROM[1:]:
        return False
    if wd == 6 and hm < CLOSED_TO[1:]:
        return False
    return True


def bar_closes_between(after: datetime, until: datetime, gran_min: int):
    """Yield bar-close instants in (after, until], on alignment boundaries."""
    step = timedelta(minutes=gran_min)
    # Anchor on the 02:00 UTC alignment of `after`'s day, then walk forward.
    anchor = after.replace(
        hour=ALIGNMENT_ANCHOR_HOUR, minute=0, second=0, microsecond=0
    )
    while anchor > after:
        anchor -= timedelta(days=1)
    # First close strictly after `after`.
    elapsed = after - anchor
    steps = int(elapsed / step) + 1
    close = anchor + steps * step
    while close <= until:
        yield close
        close += step


def parse_watch_plist(path):
    """Extract (label, slug, granularity, [instruments]) from a watch plist.

    Parses via `plutil` rather than plistlib: the shipped templates carry XML
    comments containing `--flag` text, and `--` inside a comment is malformed
    XML that expat rejects while plutil (the authority on the format) accepts.
    """
    def extract(keypath, fmt):
        out = subprocess.run(
            ["plutil", "-extract", keypath, fmt, "-o", "-", path],
            capture_output=True,
            text=True,
        )
        if out.returncode != 0:
            return None
        if fmt == "raw":  # scalar, one line, no JSON quoting
            return out.stdout.strip()
        try:
            return json.loads(out.stdout)
        except json.JSONDecodeError:
            return None

    label = extract("Label", "raw")
    args = extract("ProgramArguments", "json")
    if not isinstance(label, str) or not isinstance(args, list):
        return None
    slug = label.rsplit("com.openthink.wickd-watch.", 1)[-1]
    try:
        watch_idx = args.index("watch")
        instruments = args[watch_idx + 2].split(",")
    except (ValueError, IndexError):
        return None
    gran = "H1"
    if "--granularity" in args:
        gran = args[args.index("--granularity") + 1]
    if gran not in GRANULARITY_MINUTES:
        return None
    return label, slug, gran, [i.strip() for i in instruments if i.strip()]


def last_tick_starts(log_path):
    """Newest watcher-tick candle START time per instrument from an out.log."""
    latest = {}
    try:
        with open(log_path, "r", errors="replace") as f:
            for line in f:
                if '"watcher-tick"' not in line:
                    continue
                try:
                    ev = json.loads(line)
                    t = datetime.fromisoformat(ev["candle_time"])
                except (json.JSONDecodeError, KeyError, ValueError):
                    continue
                inst = ev.get("instrument", "?")
                if inst not in latest or t > latest[inst]:
                    latest[inst] = t
    except FileNotFoundError:
        pass
    return latest


def job_loaded(label: str) -> bool:
    return (
        subprocess.run(
            ["launchctl", "list", label],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        ).returncode
        == 0
    )


def notify(title: str, message: str) -> None:
    # ensure_ascii=False: the default \uXXXX escapes are JSON, not AppleScript,
    # and osascript rejects them as a syntax error. Raw UTF-8 is fine.
    script = (
        f"display notification {json.dumps(message, ensure_ascii=False)} "
        f"with title {json.dumps(title, ensure_ascii=False)} "
        f'sound name "Basso"'
    )
    subprocess.run(["osascript", "-e", script], check=False)


def load_state():
    try:
        with open(STATE_PATH) as f:
            return json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        return {}


def save_state(state):
    os.makedirs(os.path.dirname(STATE_PATH), exist_ok=True)
    tmp = STATE_PATH + ".tmp"
    with open(tmp, "w") as f:
        json.dump(state, f, indent=1)
    os.replace(tmp, STATE_PATH)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument(
        "--grace",
        type=int,
        default=1200,
        help="seconds a bar close may lag before it counts as missed "
        "(default 1200 — the polling source checks H8 only every 900s)",
    )
    ap.add_argument(
        "--realert",
        type=int,
        default=3600,
        help="min seconds between repeat alerts for one continuing stall",
    )
    ap.add_argument(
        "--dry-run",
        action="store_true",
        help="report to stdout only; no notification, no state update",
    )
    args = ap.parse_args()

    now = datetime.now(timezone.utc)
    state = load_state()
    problems = []

    plists = sorted(glob.glob(os.path.join(LAUNCH_AGENTS, WATCH_PLIST_GLOB)))
    if not plists:
        print("watchdog: no wickd-watch jobs installed; nothing to check")
        return 0

    for path in plists:
        parsed = parse_watch_plist(path)
        if parsed is None:
            print(f"watchdog: could not parse {path}; skipping")
            continue
        label, slug, gran, instruments = parsed
        gran_min = GRANULARITY_MINUTES[gran]

        if not job_loaded(label):
            problems.append((f"{slug}", f"launchd job {label} is NOT loaded"))
            continue

        ticks = last_tick_starts(os.path.join(LOG_DIR, f"watch.{slug}.out.log"))
        for inst in instruments:
            key = f"{slug}/{inst}"
            entry = state.setdefault(key, {})

            last_start = ticks.get(inst)
            if last_start is not None:
                watermark = last_start.astimezone(timezone.utc) + timedelta(
                    minutes=gran_min
                )
            else:
                # No tick ever logged: baseline at first sighting so a fresh
                # install doesn't page about history it never promised to see.
                if "baseline" not in entry:
                    entry["baseline"] = now.isoformat()
                watermark = datetime.fromisoformat(entry["baseline"])

            missed = [
                c
                for c in bar_closes_between(
                    watermark, now - timedelta(seconds=args.grace), gran_min
                )
                if market_open_at(c)
            ]
            if not missed:
                entry.pop("alerted_through", None)
                entry.pop("last_alert_at", None)
                print(f"watchdog: {key} {gran} ok (processed through {watermark:%Y-%m-%d %H:%M}Z)")
                continue

            newest = missed[-1].isoformat()
            already = entry.get("alerted_through")
            last_alert = entry.get("last_alert_at")
            fresh_miss = already is None or newest > already
            stale_realert = last_alert is not None and (
                now - datetime.fromisoformat(last_alert)
            ) >= timedelta(seconds=args.realert)
            if fresh_miss or stale_realert:
                closes = ", ".join(f"{c:%a %H:%M}Z" for c in missed[-3:])
                problems.append(
                    (
                        key,
                        f"{inst} {gran}: {len(missed)} bar close(s) not "
                        f"processed (latest: {closes}) — watcher may be blind",
                    )
                )
                entry["alerted_through"] = newest
                entry["last_alert_at"] = now.isoformat()
            else:
                print(f"watchdog: {key} still stalled (alert throttled)")

    for key, msg in problems:
        print(f"watchdog: PROBLEM {key}: {msg}")
        if not args.dry_run:
            notify("wickd candle watchdog", msg)

    if not args.dry_run:
        save_state(state)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
