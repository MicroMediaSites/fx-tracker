---
name: wickd-chart
description: Read a stock or forex chart with the wickd CLI. Use for chart analysis, market reads, setups, "how does <symbol> look", "compare this chart to <strategy/playbook>", earnings or calendar context, or any trading question answered from wickd data (chart bars and indicators, playbooks, quotes, lists, earnings, economic calendar, AI feed, trading journal). Read-only; never trades.
---

# wickd chart reads

You are reading markets for Matt from a terminal, with the same data the wickd
app's chart panel uses. Every command below prints one JSON object on stdout
(add `--pretty` to indent it). Errors print `{"error":{...}}`: exit 1 means
no data or a history failure, 2 missing keys, 3 an OANDA failure, 4 a bad
symbol or argument; for anything else, read `error`. On exit 2, say
which login is missing (Matt runs `wickd alpaca login` / `wickd login`
himself) and carry on with what you have.

## The chart

```sh
wickd chart context AAPL --tf 1d --bars 60      # stock
wickd chart context EUR_USD --tf 1h --bars 60   # forex pair (EUR/USD works)
wickd chart context SPY --tf 5m --extended      # include pre/post-market bars
```

`--tf` is 5m, 15m, 1h, 1d (default), 1w or 1mo; `--bars` 1..1000 (default 60).
You get exactly what the chart draws: `bars` oldest first (OHLCV plus
`ichimoku`: `tenkan`, `kijun`, `span_a_plotted`/`span_b_plotted` = the cloud
drawn at that bar, `span_a_computed`/`span_b_computed`, `chikou`), `ma50` and
`ma200` on daily bars, and `cloud_ahead` (the 26 projected cloud slots).
Values only, no verdicts; the reading is yours. Pull a higher timeframe too
when the trend matters (e.g. 1d for context, 1h or 15m for the entry).

## Playbooks (Matt's strategies)

```sh
wickd playbook ls                     # name, title, size
wickd playbook search tenkan close    # sections containing every word
wickd playbook show rahagod           # full text
```

Look a playbook up when the conversation calls for one ("compare this to
rahagod", "is this a setup?"), not by default. Read it as **loose guidance**:
the spirit of the method. Say how well the chart fits that spirit, what fits,
what doesn't, and whether the misses matter. Never grade it as a pass/fail
checklist; a strict reading is how a strategy ends up never trading.

## Other read-only sources

| Need | Command |
|---|---|
| Daily price, prev close, change (stocks only) | `wickd quote NVDA AMD --days 20` |
| OANDA candles + indicators (pairs, any granularity) | `wickd candles EUR_USD --granularity H4 --count 200 --indicators ema:20,rsi:14` |
| What a symbol is, and which lists it's on | `wickd info NVDA` |
| Matt's plan lists (note + plan per symbol) | `wickd list ls`, then `wickd list show thu-11-19` |
| Who reports earnings, when, consensus EPS | `wickd earnings upcoming --days 7 --lists` (also `--symbols`, `--min-cap`) |
| The earnings slate (why watched, expectation) | `wickd earnings slate ls --reactions` (`--all` for older) |
| What was actually reported | `wickd earnings results show NVDA` |
| Same-day results headlines (fetched and cached, like `info`) | `wickd earnings headlines NVDA` (`--date`, `--session`) |
| Economic calendar | `wickd calendar upcoming --days 7 --currencies USD --min-impact high` |
| AI market-awareness feed | `wickd feed list --limit 20` |
| Forex client positioning (sentiment) | `wickd books EUR_USD` |
| Trading journal: the day's setting, plan, rules | read `~/trading-journal/YYYY-MM-DD.md` |

Lists are named per trading day (`thu-11-19`); `wickd list ls` shows which
exist. The journal is Matt's: read the latest day for open positions, his
rules and his market filter before giving a read.

For news, sentiment and anything wickd doesn't hold (analyst moves, the story
behind a gap, sector mood), use web search and say where it came from.

## Giving a read

- Gather what the question needs: the chart, a second timeframe, the quote,
  the plan entry, upcoming catalysts (earnings, calendar), the news.
- Give a **read, not a verdict**: what you think is likely, the reasons
  (levels with real prices, indicator positions, catalysts, sentiment), and
  **what would change your mind** (the price or event that breaks it).
- Judgment beats rules: sentiment and context a chart doesn't hold count.
- Quote prices from the data, never from memory. Times are Mountain.

## Boundary: read-only, always

Never run anything that trades, approves, orders, logs in or writes. If a
wickd command isn't named in this skill, ask Matt before running it. For
example, never run `wickd trade …`, `approve`, `login`/`logout`,
`alpaca login`/`logout`, `list add|set|rm|clear|delete`,
`earnings slate add|set|rm`, `earnings sync`, `earnings results sync`,
`calendar sync`, `playbook add|rm`, `alert …`, `queue …`, `feed tick`,
`feed ask`, `watch`, `stream`, `view`, `tui`, `books --store`. Don't edit the
journal or any file under `~/.wickd/`. If a read suggests a trade or a plan
change, say so and let Matt act on it. If a store looks stale (no earnings
or calendar rows), tell him which sync to run rather than running it.
