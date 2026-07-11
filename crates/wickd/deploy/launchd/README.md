# Supervising the wickd hub + autonomous watchers with launchd (macOS)

An autonomous paper-trading setup has to survive crashes, reboots, and
unattended weekends. `wickd stream` (the socket hub) and `wickd watch --auto`
(an autonomous practice executor) are otherwise foreground processes with no
restart story — the hub dies with its process and a crashed watcher stays down.
This directory supervises both with macOS **launchd** so they start
automatically, restart on crash, and survive reboots (AGT-629).

> **Target host / status.** This is intended for the always-on Mac (the Mac
> Studio M3 Ultra, arriving ~Aug 2026). It is config + docs only and is **gated
> on that machine arriving** — actually loading the jobs into a live user
> session is the human launch step (**AGT-633**), not something the build does.
> Do not `launchctl load` this on a dev laptop you reboot/sleep.

## The jobs

| Job | Label | Command it supervises |
|-----|-------|-----------------------|
| **Stream hub** (singleton) | `com.openthink.wickd-stream` | `wickd stream <instruments> --env <e> --account <a>` |
| **Autonomous watcher** (one per strategy) | `com.openthink.wickd-watch.<slug>` | `wickd watch <strategy> <instruments> --granularity <g> --env practice --account <a> --units <n> --auto` |
| **Books collector** (singleton, periodic one-shot) | `com.openthink.wickd-books` | `wickd books <instruments> --store --env <e> --account <a>` every `StartInterval` seconds |

**The books collector is not a daemon.** launchd fires it on an interval
(default 1200s — OANDA's 20-minute book-snapshot cadence), it appends any new
order/position-book snapshots to `~/.wickd/books.db` (idempotent
`INSERT OR IGNORE`), and exits. It has no `KeepAlive` — a failed run just
waits for the next tick.

**One hub, N watchers.** The hub holds a single OANDA price subscription and
fans it out over `~/.wickd/stream.sock`. Each watcher probes that socket at
startup ([`crate::stream_hub::stream_socket_path`] → `~/.wickd/stream.sock`) and
drives its hub-covered instruments off the shared feed instead of opening a
second subscription. **No hard start-order dependency:** if the hub is down when
a watcher starts, the watcher degrades safely to its own direct OANDA sources
and re-attaches to the hub on its next (re)start. So launchd doesn't need to
sequence the two.

**Parameterized per strategy.** The watch template is installed once per
strategy/account combination; the `<slug>` (default `<strategy>-<account>`)
namespaces its Label and its log filenames, so many autonomous watchers coexist
as independent launchd jobs — each restarted independently by launchd.

## Why LaunchAgent (not LaunchDaemon)

We ship **LaunchAgents** (per-user, installed to `~/Library/LaunchAgents`, run
in the user's GUI session at **login**). Both `wickd stream` and `wickd watch`
read the OANDA **API key from the user's login keychain** (written by
`wickd login`) plus the account id from `~/.wickd/config.json` — both are
user-scoped. Because a LaunchAgent runs inside the user's login session, it can
read the login keychain **with no password prompt and no hang**.

- **"Survives reboot"** for a LaunchAgent means: on the always-on box the
  monitoring user is auto-logged-in, so the agents (re)start at login after a
  reboot — and the login keychain unlocks at that login, so the key is readable.
- **LaunchDaemon caveat:** a system LaunchDaemon runs with **no user login
  session**, so it **cannot reach the user's login keychain** — a keychain read
  from that context fails (or, with a UI present, can prompt/hang). That is
  exactly why we use a LaunchAgent. If you ever must run with no user logged in,
  you'd move the key to the **System keychain** and adjust; the simple per-user
  LaunchAgent avoids that. Crash supervision (`KeepAlive`/`RunAtLoad`) is
  identical either way.

## What the plists do

Shared keys (both templates):

| Key | Effect |
|-----|--------|
| `RunAtLoad` = true | start as soon as the agent loads (at login) |
| `KeepAlive` `{ SuccessfulExit = false }` | **restart on crash / non-zero exit**, but stay down after a clean stop |
| `ThrottleInterval` = 30 | avoid hammer-restarting in a crash loop |
| `ProcessType` = Background | low-priority background service |
| `StandardOutPath` / `StandardErrorPath` | per-job logs (see below) |
| `WorkingDirectory` / `HOME` | so `~/.wickd/*` + the login keychain resolve |

The watch template additionally **hardcodes `--env practice`** (not a
placeholder): a supervised job can never arm live autonomous execution. This is
belt-and-braces — `wickd watch --auto` also refuses `--env live` at startup
(exit 3) and the guarded auto path fails closed on live regardless. A live order
always needs an interactive `wickd approve --live` keystroke and is never
supervised.

The templates carry `__PLACEHOLDER__` tokens (`__WICKD_BIN__`, `__HOME__`,
`__LOG_DIR__`, `__ACCOUNT__`, `__INSTRUMENTS__`, and — for watch — `__LABEL__`,
`__SLUG__`, `__STRATEGY__`, `__GRANULARITY__`, `__UNITS__`) that `install.sh`
substitutes. They are **not loadable as-is**, and **no secret is stored in a
plist** — the API key is read from the keychain at runtime.

## Logs

- **Stream:** `~/Library/Logs/wickd/stream.out.log` (NDJSON prices) /
  `stream.err.log` (reconnects, errors).
- **Watcher `<slug>`:** `~/Library/Logs/wickd/watch.<slug>.out.log` (NDJSON
  signal stream **plus** autonomous-execution events — `auto-place`,
  `auto-close`, position adoption) / `watch.<slug>.err.log`.
- **Books collector:** `~/Library/Logs/wickd/books.out.log` (one JSON summary
  per run: stored/skipped counts) / `books.err.log`.

`install.sh` creates `~/Library/Logs/wickd` (launchd will not create the log
directory for you). For rotation, drop in the shipped
[`wickd.newsyslog.conf`](./wickd.newsyslog.conf) (see comments in that file).

## Prerequisites

1. `wickd` installed and on `PATH` (or know its absolute path). Prefer a stable
   **release-installed** binary (not a debug build that changes every rebuild),
   so the one-time keychain grant sticks (see Credentials).
2. OANDA practice credentials stored once for each account you'll watch:
   ```sh
   wickd login --env practice                 # the shared / default token
   wickd login --env practice --account h004  # a dedicated per-account token (optional)
   ```
3. For a scripted strategy, its `.rhai` lives under `~/.wickd/strategies/`.

## Install

```sh
cd crates/wickd/deploy/launchd

# 1) The shared hub (once). Instruments default to EUR_USD,GBP_USD,USD_JPY.
./install.sh stream "EUR_USD,GBP_USD,USD_JPY" --account h004

# 2) One autonomous watcher per strategy (repeatable).
./install.sh watch rsi "EUR_USD,GBP_USD" --account h004 --granularity H1 --units 1000
./install.sh watch h004_reversion "EUR_USD" --account h004 --slug h004-eurusd

# 3) The books collector (once). Basket defaults to the 8 USD majors + crosses;
#    interval defaults to 1200s. Override either: ./install.sh books "EUR_USD" --interval 3600
./install.sh books
```

`INSTRUMENTS` is a single comma-separated token (`clap` splits it) or `all`.
Each install renders the template, validates it with `plutil -lint`, copies it
to `~/Library/LaunchAgents/`, creates the log dir, and `launchctl bootstrap`s it
(falling back to legacy `launchctl load -w`).

Pass `--dry-run` to render + validate + print the plist **without** installing
or loading — the check the AGT-629 smoke test runs, and a safe way to preview a
job on a machine you don't want to load it on:

```sh
./install.sh watch rsi "EUR_USD" --account h004 --wickd /usr/local/bin/wickd --dry-run
```

### Scripted strategies (`--set` overrides)

The template runs a strategy by name/path with its default parameters. To
override a script's `@parameters`, edit the installed plist and add `--set`
pairs to `<ProgramArguments>` (each `--set` and its `id=value` are two separate
`<string>` elements), then reload the job.

## Credentials

There is **no master password and no secret in a plist**. `wickd` reads the
OANDA API key straight from the user's **login keychain**, resolving the
account's **dedicated** keychain item if it has one, else the **shared-token**
environment-level item (AGT-625) — so one login can back every `--account`
watcher. The only setup is logging in once (above).

For the agents to read the key unattended, two things must hold (both true on a
normal always-on box):

1. **The login keychain is unlocked** — it unlocks automatically when the
   monitoring user logs in (auto-login covers reboots).
2. **The `wickd` binary is allowed to read the item** — macOS gates keychain
   reads by code signature. The first read may pop a one-time "wickd wants to
   use the keychain" dialog; click **Always Allow**. Because a shared token
   backs every account, that single grant covers all the watchers. Use a stable
   **release-installed** `wickd` so the grant sticks across rebuilds.

## Verify

```sh
launchctl list | grep wickd                          # labels + last exit status
tail -f ~/Library/Logs/wickd/stream.err.log          # hub startup + reconnects
tail -f ~/Library/Logs/wickd/watch.<slug>.out.log    # signals + auto-execution
```

A `0` (or `-` before first run) in the status column of `launchctl list` is
healthy; a repeating non-zero status with rising PID churn means a crash loop —
check the `.err.log` (often a missing keychain entry, a locked login keychain,
an ungranted keychain prompt, or bad credentials).

## Restart + reconcile (AGT-629 AC2)

The point of `KeepAlive { SuccessfulExit = false }` is that a **crashed or
killed watcher is restarted by launchd with no manual action**. Verify it:

```sh
# 1. Find the running watcher and note its PID.
launchctl list | grep com.openthink.wickd-watch

# 2. Kill it (SIGKILL simulates a crash; a clean SIGTERM would NOT restart).
kill -9 <pid>

# 3. Within ThrottleInterval (30s) launchd respawns it — the PID changes:
launchctl list | grep com.openthink.wickd-watch

# 4. Watch the restart adopt its open positions in the log:
tail -f ~/Library/Logs/wickd/watch.<slug>.out.log
```

On that restart the watcher re-attaches to the hub and, via **AGT-628**
(startup open-position reconciliation, now merged), fetches the account's open
positions and **adopts** any on a watched instrument — seeding per-instrument
position state so duplicate entries stay suppressed and the strategy's close
logic resumes after indicator warmup. Each adoption is emitted on the signal
stream as an `"event": "auto-position-adopted"` NDJSON line in the `.out.log`
and recorded in the audit log; open positions on instruments **not** in the
watchlist are reported at startup and never touched. So the kill in step 2 above
shows up in step 4's log as the restarted process re-adopting the position it
was holding — no manual action. This ticket (AGT-629) delivers the launchd
supervision that triggers that reconciliation on every crash-restart.

## Pinning the binary for a long-running experiment (AGT-635)

`KeepAlive` restarts exec **whatever the plist's `ProgramArguments[0]` points
at, at restart time**. If that path is a build-tree binary (or a symlink into
one, e.g. `~/.cargo/bin/wickd → target/release/wickd`), any rebuild — or a
workspace rename mid-conversion — silently swaps the binary under the running
experiment: the next crash-restart execs a different (possibly half-built)
`wickd` than the one the experiment launched with. For a pre-registered
evaluation that must run unchanged for months, pin the binary before touching
the build tree:

```sh
# 1. Copy the known-good running binary to a stable path outside any build tree.
mkdir -p ~/.wickd/bin
cp "$(readlink -f ~/.cargo/bin/wickd)" ~/.wickd/bin/wickd-<experiment>
shasum -a 256 ~/.wickd/bin/wickd-<experiment> "$(readlink -f ~/.cargo/bin/wickd)"  # must match

# 2. Point each job's plist at the pin (back up the originals first).
#    Edit ProgramArguments[0] in ~/Library/LaunchAgents/com.openthink.wickd-*.plist
#    to /Users/<you>/.wickd/bin/wickd-<experiment>, then plutil -lint each.

# 3. Reload each job so launchd picks up the new path — editing the plist alone
#    does NOT re-read it; a crash-restart still uses the loaded (old) config.
launchctl bootout gui/$(id -u)/com.openthink.wickd-stream
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.openthink.wickd-stream.plist
# …repeat for each watcher label…

# 4. Verify: status 0 and the *pinned* path in the process table.
launchctl list | grep wickd
ps -axo pid,args | grep '\.wickd/bin/'
```

The copy is byte-identical, so the restart changes nothing about the
experiment's behavior; startup reconciliation (AGT-628) re-adopts any open
positions as on any supervised restart. Because the pin is a stable path, the
one-time keychain grant continues to hold (see Credentials).

**Unpin / migrate (after the experiment ends):** repoint each plist back at
the release-installed `wickd`, `bootout` + `bootstrap` each job, verify
healthy, then delete `~/.wickd/bin/wickd-<experiment>`. Do not migrate a
running pre-registered experiment onto a new binary mid-eval — that is an
intervention the experiment's registration must explicitly allow.

## Uninstall

```sh
cd crates/wickd/deploy/launchd
./uninstall.sh stream                 # stop + remove the hub
./uninstall.sh watch h004-eurusd      # stop + remove one watcher (by slug)
./uninstall.sh books                  # stop + remove the books collector
./uninstall.sh --all                  # stop + remove every wickd job
./uninstall.sh --all --purge-logs     # also delete ~/Library/Logs/wickd
```
