# Account windows

> Landed by **AGT-1128..1133** (project `wickd-account-windows`). Adds a
> per-account "since baseline" window and a custom date range to `wickd trade
> glance`/`history` and to the Live Monitor's account tiles, alongside the
> existing calendar presets.

## What it is

Every account figure on the dashboard — the P&L hero, each tile, the
trade-history drill-down — is a sum of closed trades over **a window**: a
start instant and an end instant. Five presets choose that window:

| Preset | Start | End |
|---|---|---|
| **since baseline** | the account's own recorded baseline (see below) | now |
| **today** | your local midnight | now |
| **7d** | 7 days back | now |
| **30d** | 30 days back | now |
| **custom…** | a date you pick | a date you pick (inclusive to you) |

`today`/`7d`/`30d` are the ordinary calendar questions: *was today
profitable, how's the week/month going*. **since baseline** answers a
different question — *what has this strategy done since it started* — and
it's the one worth understanding, because it behaves differently from the
other four.

## Why "since baseline" is per account

A calendar window is one shared instant applied to every account. Since
baseline is **not**: each account's window starts at *its own* recorded
baseline, so two tiles on screen at once can legitimately cover different
spans — one account started an experiment yesterday, another three weeks
ago, and "since baseline" shows each of them their own history rather than
forcing both onto the same shared start. That's the point of the feature,
not an inconsistency to squint at.

A baseline is recorded with:

```sh
wickd trade baseline set --account h004
```

which snapshots the account's current OANDA balance (or pass `--balance` to
record a specific figure) as its experiment's start. `wickd trade report` and
`wickd trade history` already measured from an account's baseline before this
project; `--since-baseline` brings the same per-account start to `glance`, so
the dashboard's ladder-wide view can use it too.

## "no baseline" is not zero

An account with no recorded baseline can't be measured against a start that
doesn't exist. Under `--since-baseline` its row reports null realized P&L,
null trade counts, and `"note": "no baseline recorded"` — never `$0.00`.
`$0.00` means "this account traded flat"; a tile that hasn't been baselined
has traded *unmeasured*, and reporting zero would silently claim otherwise.

On the dashboard this renders as a muted **no baseline** tile, and it's
excluded from the hero total (a total that folded in an unmeasured account
would be counting something that was never counted). Hover the tile for the
fix — it's the same one-liner as above:

```sh
wickd trade baseline set --account <name>
```

## The CLI flags

`wickd trade glance` gained `--since-baseline`, and both `glance` and
`wickd trade history` gained `--to`. The window either flag produces is
**`[start, to)`** — the start instant is included, the end instant is
excluded. A trade that closed exactly at `--to` falls in the *next* window,
not this one.

```
--since-baseline     Start each account's window at ITS OWN recorded baseline
                      (its experiment start) instead of one shared instant.
                      Mutually exclusive with --since and --days — but NOT
                      with --to: the two compose, closing every account's own
                      window at the same shared instant.

--to <TO>             Window end — an ISO date (YYYY-MM-DD) or RFC3339
                      instant. Defaults to now.
```

`--since-baseline` composes with `--to` (one shared end, each account's own
start) but conflicts with `--since`/`--days` (clap rejects passing both) —
there's no single shared start to reconcile with a per-account one.

**Example — every account's performance since it was baselined:**

```sh
wickd trade glance --since-baseline
```

Each row carries `window_start` (the RFC3339 instant that account's window
opened, or `null` if it has no baseline) and `window_source` (`"baseline"` |
`"since"` | `"days"`), so a consumer never has to infer which input decided
the window. The top-level `since` field is `null` under `--since-baseline` —
there is no single shared start to report there.

**Example — close every account's window at a fixed instant** (useful for a
reproducible end-of-day snapshot rather than "now"):

```sh
wickd trade glance --since-baseline --to 2026-08-25T00:00:00Z
```

**Example — one account's trade-by-trade history over an explicit range:**

```sh
wickd trade history --account h004 \
  --since 2026-08-01 --to 2026-08-15
```

`history` already defaults `--since` to the account's baseline, so the
plain per-account case (no explicit range) needs no flags at all —
`wickd trade history --account h004` is "since baseline" by default.

## The desktop picker

The Live Monitor's account section has a window picker —
**since baseline · today · 7d · 30d · custom…** — and whichever one is
selected drives the hero total, every tile, and the trade-history drill-down
together; there's one window for the section, not one per tile (a per-tile
override is deferred — see the project README if you're wondering why a tile
can't be pinned to its own window yet).

**custom…** takes two calendar dates. The end date you pick is **inclusive**
— "Aug 1 – Aug 24" means trades through the end of Aug 24 count — even
though under the hood it's sent to the CLI as the exclusive midnight that
starts Aug 25, the same instant math `--to` uses everywhere else.

The picker's choice persists across restarts (`localStorage`). On a fresh
install it defaults to **since baseline** if any configured account has one,
otherwise **today** — the ladder-wide question only makes sense once there's
a ladder to ask it about.

Each tile also shows its own window's start in its footer (e.g. "since Aug
25", exact instant on hover) — the reminder that under "since baseline" that
start can differ from the tile next to it.
