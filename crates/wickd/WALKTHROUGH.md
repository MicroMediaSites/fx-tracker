# wickd — a guided tour

A hands-on walk through everything `wickd` can do, meant to be followed **together by
you and an agent**: the agent runs each command and narrates what came back; you watch the
output scroll by and get a feel for the tool. It's a demo, not a test — nothing here asserts
pass/fail, it just shows the features off, roughly in the order they build on each other.

> **The shape of the thing.** `wickd` is a headless, agent-first OANDA CLI — the *hands and
> eyes*. It pipes clean market data in (candles, instruments, live prices) and sends orders
> out; the *brain* (strategy, reasoning) is the agent wrapping it. Everything is JSON by
> default so an agent can consume it; add `--pretty` for human-readable indented JSON, which
> we'll use throughout this tour.

## How to run the tour

- Build once, then call the binary directly so you're seeing the real thing:
  ```sh
  cd ~/Development/fx-tracker
  cargo build --release -p wickd
  export WICKD=~/Development/fx-tracker/target/release/wickd
  "$WICKD" --help          # the full verb list — this is your map
  ```
- **Credentials:** the live bits use an OANDA **practice** account (read-only data + paper
  trading, no real money). Check `"$WICKD" login --status`. If nothing's configured, run
  `"$WICKD" login --env practice` once (it prompts for your API key + account id; the key
  goes in the OS keychain, never a file).
- **Market hours:** the live-price stops (stream / dashboard / watch / alerts) only come alive
  when the FX market is open (≈ Sunday 22:00 → Friday 22:00 UTC). Off-hours everything still
  runs, you just won't see ticks flow. The offline stops (candles, backtest, validate) work
  anytime.
- **macOS: no `timeout`.** The live stops wrap long-running commands in `timeout N …`, but
  macOS ships no `timeout` binary (GNU coreutils calls it `gtimeout`). Either
  `brew install coreutils` and use `gtimeout`, or paste this portable stand-in into your
  shell once before the tour:
  ```sh
  command -v timeout >/dev/null || timeout() {
    local secs=$1; shift
    "$@" & local pid=$!
    (sleep "$secs"; kill "$pid" 2>/dev/null) &
    wait "$pid" 2>/dev/null
  }
  ```
- **Any command takes `--help`** — that's the honest source of truth for flags on your build.
  Lean on it.

Pick and choose — each stop stands alone. But the order below tells a story: *look at the
market → build a strategy → prove it on history → go live → watch for signals → trade, safely.*

---

## Stop 1 — Look around the market

Start with the raw material. What can we trade, and what does the price history look like?

```sh
"$WICKD" instruments --env practice --pretty        # every tradeable instrument
"$WICKD" candles EUR_USD --granularity H1 --count 20 --pretty
```

**What you'll see:** a list of instruments (EUR_USD, GBP_USD, …), then 20 hourly OHLC candles
for EUR_USD — open/high/low/close with timestamps. This is the clean, typed data every other
feature is built on (prices are exact decimals, never floats).

---

## Stop 2 — Strategies & backtesting *(the analytical heart)*

wickd ships two built-in strategies and a real, **verified** backtest engine.

```sh
"$WICKD" strategy --help                              # see the built-ins + their params
"$WICKD" strategy run ma-crossover EUR_USD --granularity H1 --count 300 --pretty
```

**What you'll see:** the moving-average-crossover strategy evaluated over the last 300 hourly
candles, emitting its **signals** (buy/sell) as JSON. Add `--include-holds` to see the "do
nothing" bars too.

Now prove it on history:

```sh
"$WICKD" backtest ma-crossover EUR_USD --granularity H1 --count 500 --pretty
```

**What you'll see:** a full metrics block — total P&L, win rate, average win/loss, profit
factor, max drawdown, Sharpe — plus the individual trades. Every number here is checked by
hand-computed fixture tests, so it's trustworthy, not "looks about right."

### Walk-forward: the honest version

A single backtest can overfit. Walk-forward splits history into rolling **in-sample** (train)
and **out-of-sample** (test) windows, re-optimizing parameters on each train window and scoring
on the untouched test window — so you can *see* the overfitting gap.

```sh
"$WICKD" backtest ma-crossover EUR_USD --granularity H1 --count 1500 \
    --walk-forward --is-size 250 --oos-size 100 --pretty
```

**What you'll see:** several windows, each with its own in-sample vs out-of-sample metrics, and
an aggregate. When the OOS numbers hold up near the IS numbers, the edge is probably real; when
they collapse, you were curve-fitting. That contrast is the whole point.

---

## Stop 3 — Author a strategy in your own words *(the agent loop)*

You don't hand-write `.rhai` scripts — you talk to an agent, it writes the script, and wickd
gives it clean JSON surfaces to validate and compare variants. Here's that loop by hand.

Drop a tiny strategy at `~/.wickd/strategies/tour.rhai` (copy the shape from
`crates/wickd-core/tests/golden_scripts/01_minimal.rhai`, or the `on_candle` +
`@parameters` contract documented in [`STRATEGY_ABI.md`](./STRATEGY_ABI.md)), then:

```sh
"$WICKD" strategy validate tour --pretty      # → { valid: true, score, errors: [], metadata }
"$WICKD" backtest tour EUR_USD --granularity H1 --count 500 --pretty
```

**What you'll see:** `validate` returns a clean, machine-actionable verdict (no prose to parse) —
and if you deliberately break the script, it returns `valid: false` with structured errors at a
normal exit, never a panic. The scripted strategy then backtests with the **exact same metric
shape** as a built-in, so an agent can generate 2–3 variants and compare them apples-to-apples.
This is what lets "make the RSI exit tighter" become a real, measured change instead of a guess.

---

## Stop 4 — Live prices *(the real-time substrate)*

```sh
# runs until Ctrl-C — here we let it run ~15s then stop
timeout 15 "$WICKD" stream EUR_USD,GBP_USD
```

**What you'll see (market open):** a firehose of NDJSON price ticks — one JSON object per line,
each a `price-update` with instrument / bid / ask / spread / time. This is the single upstream
OANDA connection everything live is built on. (`stream all` streams every instrument; a
`~/.wickd/watchlist.json` can name reusable lists.)

---

## Stop 5 — One stream, many watchers *(the socket hub — a highlight)*

Here's a neat bit. A single `wickd stream` doesn't just print to your terminal — it stands up a
local **socket hub** at `~/.wickd/stream.sock` and fans that one OANDA subscription out to any
number of consumers. So a dashboard, a strategy watcher, and an agent can all read the *same*
live feed without each opening its own connection.

**See the fan-out:** in one terminal start a stream and leave it running:

```sh
"$WICKD" stream EUR_USD,GBP_USD,USD_JPY
```

In two *other* terminals, attach raw readers to the socket:

```sh
nc -U ~/.wickd/stream.sock      # (or: socat - UNIX-CONNECT:$HOME/.wickd/stream.sock)
```

**What you'll see:** both readers receive the *identical* line stream, live. Kill the stream and
notice `~/.wickd/stream.sock` disappears — it lives exactly as long as the stream process.

### The dashboard *(a terminal cockpit over the hub)*

```sh
# With NO stream running — see the guardrail:
"$WICKD" dashboard          # clean error: "start `wickd stream` first" (it won't open a rival feed)

# With a stream running in another terminal:
"$WICKD" dashboard          # a live ratatui table: one row per instrument, bid / ask / spread, updating in place
```

**What you'll see:** a full-screen terminal dashboard of the watchlist, updating as ticks
arrive — and `q` or Ctrl-C restores your terminal cleanly. Notice it *attaches to the running
hub* rather than starting its own subscription; with no hub up, it tells you so instead of
quietly competing.

---

## Stop 6 — Watch a strategy against the live market *(and share the stream)*

`wickd watch` runs a strategy against live candles and emits signals — a monitoring daemon that
**never trades**. The nice part: if a `wickd stream` hub is already running, `watch` *attaches to
it* and shares that one subscription instead of opening a second one.

**See both paths:**

```sh
# No hub running → watch opens its own subscription (the always-safe fallback):
timeout 20 "$WICKD" watch ma-crossover EUR_USD --granularity M1 --count 200

# Now start a stream in another terminal first…
"$WICKD" stream EUR_USD,GBP_USD
# …then run the watcher on a covered instrument — it attaches to the shared hub feed:
timeout 40 "$WICKD" watch ma-crossover EUR_USD --granularity M1 --count 200
```

**What you'll see:** in the second case the watcher logs that it attached to the hub and folds
the shared tick stream into candles itself — no second OANDA subscription. Ask for an instrument
the hub *isn't* streaming (`watch ma-crossover EUR_USD,USD_JPY …` against a EUR_USD/GBP_USD
stream) and it splits the difference: EUR_USD off the shared feed, USD_JPY off its own direct
source. (M1 granularity so a candle actually closes while you're watching.)

---

## Stop 7 — Alerts & the agent queue *(epic #2)*

### Price-level alerts

```sh
# arm an alert a few pips from the current mid — the live feed prints bid/ask only,
# so compute it yourself: mid = (bid + ask) / 2
"$WICKD" alert add --instrument EUR_USD --price 1.0900 --direction cross-up --source mid --rearm 5
"$WICKD" alert list --pretty                     # armed, with status
timeout 30 "$WICKD" alert run                    # watches the live feed; fires (NDJSON) on a cross
"$WICKD" alert remove <id>                        # tidy up
```

**What you'll see:** the alert sits armed; when price crosses your level it fires a line (with a
hysteresis band so it doesn't chatter across the level). Edge-triggered, not spammy.

### Strategy-signal alerts → a queue an agent drains

```sh
# human-readable signal feed — one clean line per Buy/Sell, firehose suppressed:
timeout 30 "$WICKD" watch ma-crossover EUR_USD,GBP_USD --granularity M1 --format human

# the durable queue those signals land in, and the bridge to a trade proposal:
"$WICKD" queue list --pretty
"$WICKD" queue promote <queue-entry-id>          # turns a signal into a pending proposal
"$WICKD" pending --pretty                          # …which now shows up here
```

**What you'll see:** the `human` format turns the signal firehose into readable one-liners; the
queue is where an agent (or you) picks signals up asynchronously; and `promote` is the bridge
from "a strategy fired" to "a trade is proposed for approval" — which leads straight into the
last stop.

---

## Stop 8 — Trading, and the safety model *(epic #4 — the important part)*

wickd can place orders, but it's built so nothing hits the broker by accident. Everything is
**paper (dry-run) by default**; a real order takes deliberate arming.

### Look, then paper-trade

```sh
"$WICKD" trade account --pretty         # balance, NAV, margin
"$WICKD" trade positions --pretty
"$WICKD" trade orders --pretty

# a market order WITHOUT --live = paper: it shows the would-be order, contacts nothing:
"$WICKD" trade place --instrument EUR_USD --units 100 --pretty
"$WICKD" trade positions --pretty        # …still empty. Nothing was placed.
```

### Limit / stop entries (4-outcome classified)

```sh
"$WICKD" trade place --instrument EUR_USD --units 100 --type limit --price 1.0800 --tif gtc --pretty   # paper
"$WICKD" trade place --instrument EUR_USD --units 100 --type stop  --price 1.0950 --pretty              # paper
```

**What you'll see:** the dry-run shows the resting-order shape (type / price / time-in-force)
without submitting — market, limit, and stop entries each classified correctly.

### The trust ladder: propose → approve → *keystroke* → submit

```sh
# watch in semi-auto records tradeable signals as PROPOSALS (still never trades):
timeout 25 "$WICKD" watch ma-crossover EUR_USD --granularity M1 --semi-auto
"$WICKD" pending --pretty                 # proposals waiting for you
"$WICKD" approve <signal_id> --pretty     # WITHOUT --live = paper approval, contacts nothing
"$WICKD" audit --pretty                    # append-only log of every decision (paper ones included)
```

**What you'll see:** signals become proposals, proposals can be approved on paper, and every
decision is written to an append-only audit log **before** anything could be submitted.

### The live keystroke gate — worth seeing yourself

This is the safety centerpiece, and it's deliberately **human-only** — an agent physically
can't arm it. In *your own terminal* (practice account, 1 unit):

```sh
"$WICKD" trade place --instrument EUR_USD --units 1 --live --env practice
```

**What you'll see:** it stops and demands an **interactive keystroke** before it will submit —
and `--yes` does *not* satisfy it (a live submit requires a real TTY confirmation). Press the key
and a 1-unit practice order goes in; `"$WICKD" trade positions` shows it; `"$WICKD" trade close
--instrument EUR_USD --side long --live` (another keystroke) closes it. That keystroke wall is what makes the whole
"agent proposes, human disposes" model safe: the agent can do everything up to the trigger, but
the trigger is yours.

---

## Where to go next

- [`README.md`](./README.md) — the full verb reference.
- [`STRATEGY_ABI.md`](./STRATEGY_ABI.md) — the versioned `.rhai` strategy contract + the
  agent authoring loop (generate → validate → backtest → iterate).
- Everything emits JSON without `--pretty` — that's the surface an agent actually consumes;
  `--pretty` is just for tours like this one.
