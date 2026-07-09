# Rhai dialect compatibility report — app vs wickd (AGT-637)

**Date:** 2026-07-06 · **Engine at:** `main` @ `7d82883` · **wickd ABI:** `crates/wickd/STRATEGY_ABI.md` v4

Scope: every API/semantic difference between the desktop app's scripted-strategy
dialect and the wickd `STRATEGY_ABI.md` contract, a classification of all 33
archived CandleSight strategies (Matt's non-archived rows in the prod dump), and
a unification recommendation the strategy-store ticket can implement directly.

## TL;DR

- **There is exactly one Rhai engine.** Both the app (`src-tauri`) and wickd
  execute scripts through the same code:
  `crates/wickd-core/src/backtest/scripted_strategy.rs`
  (`src-tauri/src/lib.rs:13` re-exports `wickd_core::backtest`; both
  crates pin `rhai = "1.21"` with `sync, decimal, no_float`). The *language*
  never forked. What drifted is (a) **what each host injects at construction**
  and (b) **what each side's authoring documentation claims**.
- **All 9 archived scripted strategies run under the wickd ABI as-is** —
  verified empirically, not just by reading (see Evidence).
- Of the 24 rules-DSL strategies: **19 are mechanically translatable** to
  `.rhai`, **4 need redesign** (multi-timeframe indicators — no MTF in the
  Rhai ABI), **1 is an empty shell** (drop).
- The biggest live risk is **silent degradation, both directions**: app-run
  scripts see permanent sentinels for all 6 event/surprise functions (D1/D2),
  and 11 of the 29 live `~/.wickd/strategies/*.rhai` use ABI v2–v4 functions
  the app's authoring doc doesn't even mention (D5).

## One engine, two hosts

| | app (`src-tauri`) | wickd CLI/daemon |
|---|---|---|
| Engine | `wickd_core::backtest::ScriptedStrategy` | same type, same crate |
| Rhai features | `sync, decimal, no_float` (1.21) | identical (workspace dep) |
| Entry point | `run_custom_backtest` → `ScriptedStrategy::from_script` (`src-tauri/src/commands/backtest.rs:713`) | `from_script_with_params` (`crates/wickd/src/commands/scripted.rs:156`) |
| Validation | same `validate_script` (`src-tauri/src/ai/tools.rs:1975`) | same, via `wickd strategy validate` |
| Limits | identical (`configure_engine_limits`, one function) | identical |

So `on_candle()`/`on_position_closed()`, the `@indicators`/`@parameters`
metadata format, the signal-map contract, Decimal semantics, resource limits,
and the "typos degrade to inert" convention are **byte-for-byte shared**.
Every difference below is host wiring or documentation.

## Dialect differences (complete enumeration)

### Host-injection differences (runtime semantics)

**D1 — Event calendar (ABI v3) is wickd-only.** wickd injects
`set_event_calendar` in `backtest`/`strategy run`
(`crates/wickd/src/commands/scripted.rs:167`) and in the watcher
(`crates/wickd/src/commands/watch.rs:273`). The app **never** calls
`set_event_calendar`/`set_script_event_calendar` anywhere in `src-tauri`.
Consequence: in any app backtest or app watcher, `hours_since_event()` and
`hours_until_event()` return **-1 forever** — an event-blackout script is
silently inert, indistinguishable from "no events nearby".

**D2 — Surprise feed (ABI v4) is wickd-only.** Same shape: wickd wires
`set_surprise_calendar` (`scripted.rs:173`, `watch.rs:279`); the app never
does. In the app, `surprise_z()`/`surprise_z_for()` return **-9999** and
`surprise_hours_ago()`/`surprise_hours_ago_for()` return **-1**, always.
Measured impact today: **11 of 29** live `~/.wickd/strategies/*.rhai` call at
least one v2–v4 function (5 use `surprise_z`, 2 `hours_since_event`, 6
`candle_time`, 5 `candle_hour`) — those strategies degrade silently if run
through the app.

**D3 — Parameter overrides are wickd-only.** wickd resolves `--set` overrides
via `from_script_with_params` (backtest, run, walk-forward, watch —
`watch.rs:270` `set_script_params`). The app's scripted backtest path uses
plain `from_script` (metadata **defaults only**), and the app watcher never
calls `set_script_params`. In the app, tuning a scripted strategy means
editing the script's `@parameters` defaults (the AI tool then re-syncs the DB
`parameters`/`indicators` columns *from* script metadata —
`src-tauri/src/ai/tools.rs:1985-2001`). Note `from_script_with_params`
silently ignores unknown override keys on both hosts.

**D4 — Storage & resolution differ.** wickd: explicit path → built-in
(`ma-crossover`, `rsi`) → bare name in `~/.wickd/strategies/<name>.rhai`.
App: `strategy.script_content` column in Postgres, keyed by strategy id; no
built-ins; names are display-only and **not unique** (the archive has two
distinct rows both named "Ichi w MACD Confirm - Low TF", plus 5 rows that are
byte-identical script bodies under 4 different names).

**D5 — The app's authoring doc is frozen at ABI v1.**
`queries-service/src/rhai-strategy-prompt.md` (the LLM prompt that authors app
scripts) documents exactly the 16-function March-2026 SDK. Missing: all 8
v2–v4 functions (`candle_time`, `candle_hour`, `hours_since_event`,
`hours_until_event`, `surprise_z`, `surprise_z_for`, `surprise_hours_ago`,
`surprise_hours_ago_for`). App-authored scripts therefore can't use the
current surface; wickd-authored scripts look "wrong" to the app's authoring
loop. (Given D1/D2, the app engine would only feed those functions sentinels
anyway — doc and wiring are consistently stale together.)

**D6 — `pending_order` is real but undocumented in the wickd ABI.** The
engine parses `pending_order: #{ order_type, price, expiry_bars }` on
buy/sell signals (`scripted_strategy.rs:619`, `parse_pending_order_map`,
added AGT-607 2026-06-30). The **app prompt documents it**
(`rhai-strategy-prompt.md` "Pending Orders" section);
**`STRATEGY_ABI.md` v4 does not mention it** — the signal-map table stops at
`exit_reason`. This is the one place the app doc is *ahead* of the ABI doc.
Verified: a `pending_order` script validates clean under
`wickd strategy validate` (score 90, only `no_parameters` warning).

**D7 — `select` options: the app prompt documents a form the engine drops.**
Prompt says `"options": ["a", "b", "c"]` (string array). The parser
(`parse_parameters_json`, `scripted_strategy.rs:1731`) only accepts
`{ "value": number, "label": string }` objects and **silently filters out**
anything else. Probe result: string options → `"options": []` in validated
metadata, no error, no warning. `STRATEGY_ABI.md` documents the correct
object form. (Select values are numeric by construction — `param()` returns
numbers; string-valued selects were never possible.)

**D8 — Boolean parameter defaults must be numeric.** `"default": true` parses
via `as_f64()` → `None` → **0.0** silently (probe-verified). Use
`"default": 1` / `"default": 0`. Neither doc calls this out.

**D9 — Engine evolution March→HEAD is strictly additive for language surface,
with tightened runtime guards.** The 33 archived strategies were authored
2026-02-05 → 2026-03-29 against the then-app engine
(`d798d0d:src-tauri/src/backtest/scripted_strategy.rs`, 16 registered
functions — exactly the D5 list). Nothing was removed or renamed since. New
guards that now also apply to old scripts: ~50ms wall-clock budget per script
call, array (10k) / map (1k) size caps, and the 50-consecutive-error
permanent-Hold abort with `take_abort_event()` health signaling (AGT-606).
None of the archived scripts comes near these limits (verified by execution).

**D10 — Reserved-keyword hazard (both hosts, translation-relevant).** Rhai
reserves words like `until`: `let until = ...` fails with the cryptic
`Expecting name of a variable (line N, position M)` (probe-verified; golden
`06_event_blackout.rhai` documents it). Mechanical rules→rhai translation must
sanitize generated variable names (`until`, `do`, `switch`, `import`, …).

**D11 — The rules DSL has no wickd entry point.** `wickd strategy
validate/run/backtest` resolve only `.rhai` files and built-ins; a
`strategy_type: "rules"` definition is app-only today (the shared
`MultiInstrumentWatcher` can still execute rules definitions, but no wickd
command constructs one). Also parity-relevant: **scripted strategies get no
S/R zones, pivots, or multi-timeframe candles on either host** — those are
rules-engine-only features (`src-tauri/src/commands/backtest.rs:708-710`
skips them deliberately for scripted).

## Classification of the 33 archived strategies

Set definition: `strategy` rows in the prod dump
(`~/Documents/candlesight-archive/candlesight-prod-2026-07-06.dump`) owned by
Matt's primary user with `is_archived = false` → exactly 33 rows:
9 `scripted` + 24 `rules`.

Summary: **9 as-is · 19 mechanical translation · 4 redesign · 1 drop.**

### Scripted (9) — all "runs under wickd ABI as-is"

Evidence (three independent checks, see Evidence section): `wickd strategy
validate` = `valid: true, score: 100, 0 warnings` for every script; static
call audit — every SDK call ∈ the 24 registered functions (all v1-surface:
`price`, `price_at`, `indicator`, `indicator_at`, `param`, `pip_value`,
`bar_count`, `crossed_above/below`, signal keys only
`signal/stop_loss/take_profit/rule_name/exit_reason`, no `pending_order`);
and a 300-candle execution through the exact app construction path with zero
aborts. Indicator metadata uses only shared enum types
(`ichimoku`, `macd`, `atr`, `adx`, `ema`, `daily`).

| Strategy (id) | Created | Note |
|---|---|---|
| Ichi w MACD Confirm (v2) (`a21cbe60-…`) | 2026-03-29 | richest: 12 params, 3 indicators, close-signals |
| Ichi w MACD Confirm (`a468de50-…`) | 2026-03-29 | body identical to 4 other rows (dedupe) |
| Ickimoku with MACD confirmation (`strat_1773842788707`) | 2026-03-18 | |
| Ichi w MACD Confirm - EUR/USD H1 (`strat_1774125241743`) | 2026-03-21 | |
| Ichi w MACD Confirm - Low TF (`strat_1774134783658`) | 2026-03-21 | |
| Long Ichi w MACD Confirm - EUR/USD H1 (`strat_1774134820283`) | 2026-03-21 | |
| Ichi w MACD Confirm - Low TF (`strat_1774207630938`) | 2026-03-22 | duplicate name of `…783658`, different body |
| Mean Reversion (`strat_0d48c7e7-…`) | 2026-03-24 | |
| HiLo Open (`strat_c1d8d475-…`) | 2026-03-24 | |

(The dump's 3 further scripted rows — 2 archived, 1 other-user — also
validate clean; noted for completeness, out of the 33-set.)

Only 8 distinct script bodies exist across the 9 rows ("Ichi w MACD Confirm"
== "Ichi w MACD Confirm - Low TF" `…630938` == 3 out-of-set rows). The store
migration should dedupe by content hash.

### Rules DSL (24)

Flag legend — what each strategy needs from the translation recipes below:
**RR-exit** = `risk_reward_reached`/`percent_of_tp_reached` exit (recipe R5,
needs self-tracked entry state; exact translation wants ABI v5), **regime** =
`givens: trending_up/down/ranging` (R4), **session** =
`givens: london_session/us_session` (R3), **bars** = bar-count exit (R6),
**price-cross** = price↔indicator crossover (R2).

| Strategy (id) | Created | Classification | Flags |
|---|---|---|---|
| 2023 Reverse Engineered (`6dd2d692-…`) | 2026-03-14 | mechanical translation | regime |
| 2023 Reverse Engineered (`71e0daf2-…`) | 2026-03-16 | **needs redesign** | MTF:D, regime |
| 2023 Reverse Engineered (`f79355dd-…`) | 2026-03-16 | **needs redesign** | MTF:D |
| Copy of Matt's Ichimoku (v2) (`c5196c2b-…`) | 2026-03-12 | mechanical translation | RR-exit, regime |
| Custom Ichimoku Strategy (`f405be70-…`) | 2026-02-05 | mechanical translation | RR-exit, price-cross |
| Ichimoku with MACD Confirmation (v1) (`strat_1773893088978`) | 2026-03-19 | **drop** | empty shell — 0 indicators, 0 rules |
| Intraday Momentum Trend (`76c78f87-…`) | 2026-03-14 | mechanical translation | RR-exit, session, bars |
| Intraday Momentum Trend (`70887e50-…`) | 2026-03-14 | mechanical translation | RR-exit, bars |
| London Breakout v1 (`bdb0b6b3-…`) | 2026-02-12 | mechanical translation | session, bars, price-cross |
| MACD Crossover (`e1932675-…`) | 2026-03-15 | mechanical translation | regime |
| MACD Crossover Trend Filter (`b951f2fa-…`) | 2026-03-14 | mechanical translation | RR-exit, regime |
| MTF Stress Test (`c8d5dbe8-…`) | 2026-03-16 | **needs redesign** | MTF:H4 |
| Matt's Ichimoku (`9aa21d3b-…`) | 2026-02-16 | mechanical translation | RR-exit, regime |
| Matt's Ichimoku (v3) (`strat_1773289411337`) | 2026-03-12 | mechanical translation | RR-exit, regime |
| Matt's Ichimoku (v3) (`strat_1773706694866`) | 2026-03-17 | mechanical translation | RR-exit, regime |
| Matt's Ichimoku - GBP/USD H4 (`strat_1773670448912`) | 2026-03-16 | mechanical translation | RR-exit, regime |
| Matt's Ichimoku - USD/JPY H4 (`strat_1773667217636`) | 2026-03-16 | mechanical translation | RR-exit, regime |
| Matt's Ichimoku EUR/USD H4 (`strat_1773669209231`) | 2026-03-16 | mechanical translation | RR-exit, regime |
| RSI Dip Long with Swing Low SL (`strat_1773503174918`) | 2026-03-14 | mechanical translation | RR-exit |
| RSI Dip Long with Swing Low SL (3-14) (`strat_1773514120028`) | 2026-03-14 | mechanical translation | RR-exit |
| RSI Overbought Short (`df472765-…`) | 2026-02-25 | mechanical translation | RR-exit, regime |
| Tenkan Gap Breakout (`85c0b5eb-…`) | 2026-03-15 | mechanical translation | RR-exit |
| Trend Following Base (`672d7860-…`) | 2026-03-16 | **needs redesign** | MTF:H4/M5 |
| Trendline Scalping (`12e1619b-…`) | 2026-02-12 | mechanical translation | RR-exit, price-cross |

Trigger-type census across all 24 (for sizing): `compare` ×153, `cross` ×68,
`threshold` ×29, `risk_reward_reached` ×16, `givens:ranging` ×15,
`givens:trending_up/down` ×8, `givens:london/us_session` ×4,
`time:bar_count` ×3, `percent_of_tp_reached` ×2. Risk settings used:
`risk_method/value`, `rr_ratio`, `spread_buffer_pips`,
`stop_loss_source[_short]` (swing/chandelier).

### Translation recipes (rules DSL → Rhai ABI v4)

- **R1 `compare`/`threshold`** → `indicator(id, out)` / `price(field)` +
  operators, or `crossed_{above,below}_value` where the rule means a level
  cross. Direct.
- **R2 `cross` with a price leg** — the SDK's `crossed_above(id1,out1,id2,out2)`
  is indicator↔indicator only. Price↔indicator crosses translate as two
  comparisons: `price_at("close",1) <= indicator_at(id,out,1) &&
  price("close") > indicator(id,out)`. Direct, 2 lines. (A
  `price_crossed_above(id,out)` helper would be a nice-to-have, not a
  blocker.)
- **R3 session givens** → `candle_hour()` window gates (golden
  `05_session_gate.rhai` is the pattern). Use the session bounds from the
  rules engine (`rules_triggers.rs`) so semantics match.
- **R4 regime givens** → declare `adx`, `sma(20)`, `sma(50)`, `bollinger`
  indicators and reimplement `check_trending_up/down/ranging` from
  `crates/wickd-core/src/backtest/regime_detector.rs` (defaults: ADX
  trend ≥ 25, range < 20, plus SMA-order and BB-width tests). Deterministic,
  ~15 lines shared across the 12 regime-flagged strategies.
- **R5 RR-based exits** (`risk_reward_reached`, `percent_of_tp_reached`) —
  the ABI exposes **no position state** (no entry price, no in-position
  flag). Scripts must self-track (`in_position`, `entry_price ≈` signal
  candle close, clear in `on_position_closed()`). This is an
  **approximation**: the engine fills at the next candle open, so the
  self-tracked entry is off by one gap. Exactness needs ABI v5 (below). The
  16 RR-exit strategies remain *mechanical* — the recipe is uniform — but
  flag backtest parity as approximate until v5 lands.
- **R6 bar-count exits** → self-tracked `bars_since_entry` counter
  (increment per candle while in position; reset in `on_position_closed()`).
  Same approximation caveat, minor.
- **Risk settings** → `rr_ratio`/`spread_buffer_pips`/`stop_loss_source`
  become explicit `stop_loss`/`take_profit` arithmetic in the signal map
  (swing/chandelier stops via the `swing`/`chandelier` indicators, spread
  buffer via `pip_value()`). `risk_method/value` (position sizing) stays
  host-side (`BacktestConfig`), not in-script — matches wickd.
- **Hazards**: sanitize variable names against Rhai reserved words (D10);
  emit only `{value,label}` select options (D7); numeric boolean defaults
  (D8).

**Why the 4 MTF rows are redesign, not translation:** the Rhai SDK has no
HTF indicator access and the hosts don't feed scripted strategies HTF candles
(D11). Either wait for an MTF ABI extension (new ticket) or re-express the
strategy single-timeframe (e.g. approximate a D-trend filter with long-period
EMAs on H4) — a semantic change, hence "redesign".

## Evidence (what was actually run)

All 12 scripted bodies were extracted from the prod dump
(`pg_restore --data-only --table=strategy`, PostgreSQL COPY unescaped) and:

1. **wickd parse/validate** — `target/release/wickd strategy validate <file>`
   (built at `7d82883`) on each: **12/12 `valid: true`, score 100, zero
   errors/warnings**, metadata (params/indicators) parsed.
2. **App-path execution** — a transient harness (not committed) constructed
   each via `ScriptedStrategy::from_script(script, name)` +
   `set_pip_value_for_instrument("EUR_USD")` — the literal
   `run_custom_backtest` scripted arm, sentinels and defaults exactly as the
   app — and drove 300 synthetic candles through `on_candle_extended`:
   **12/12 completed, `take_abort_event() == false` for all** (no runtime
   function-resolution errors, no limit hits). Several produced live signals
   (e.g. Mean Reversion: 23 buy / 27 sell / 24 close; HiLo Open: 7 buy /
   13 sell); the Ichimoku-confirm family mostly held on synthetic data, as
   expected for entry conditions tuned to real cloud/MACD geometry.
3. **Probes** (same binary): string-array select options → silently emptied;
   `"default": true` boolean → 0.0; `pending_order` script → valid;
   `let until` → `compile_error: Expecting name of a variable`.
4. **History diff** — `git show d798d0d:src-tauri/src/backtest/scripted_strategy.rs`
   (the engine the archived scripts were written against) vs HEAD: 16 → 24
   registered functions, zero removals/renames; limits added per D9.
5. **Golden corpus / workspace tests** — `cargo test --workspace` green at
   `7d82883` (includes `golden_script_corpus.rs`, which pins `STRATEGY_ABI.md`
   examples to `validate_script`).

## Unification recommendation (for the strategy-store ticket)

**Canonical form: one `.rhai` file per strategy, STRATEGY_ABI v4 dialect,
metadata comments as the single source of truth.** The DB/app columns
(`parameters`, `indicators`) become derived caches of script metadata — the
app's AI tool already works this way (`tools.rs` re-syncs columns from
`validate_script` output), so this codifies existing behavior rather than
changing it.

Concretely, in order:

1. **Single shared constructor.** Move wickd's host wiring
   (`crates/wickd/src/events.rs` calendar/surprise loaders + the
   `scripted.rs:156-173` construction sequence) into `wickd-core`, e.g.
   `ScriptedStrategy::for_host(script, name, overrides, instrument)` that
   always injects event calendar + surprise feed + pip value. Point wickd
   *and* `src-tauri`'s `run_custom_backtest`/watcher at it. This erases D1,
   D2, D3 structurally — the two hosts can no longer drift because there is
   nothing left to wire per-host. (The `~/.wickd/events.json` +
   `~/.wickd/calendar/` sources work unchanged for the app.)
2. **Store migration of the 33:**
   - 9 scripted → import `script_content` verbatim (they pass v4 validation
     as-is). Dedupe by content hash first (8 distinct bodies in-set; 5-way
     duplicate across the dump). Slugify names to unique store keys
     (two rows share "Ichi w MACD Confirm - Low TF").
   - 19 rules → translate with recipes R1–R6; run each through
     `wickd strategy validate` (exit-0 JSON, `valid: true` gate) as the
     acceptance check. The regime helper (R4) should be written once and
     shared.
   - 4 MTF rules → park under an `attic/mtf/` namespace with their original
     JSON attached; revisit iff an MTF ABI extension ships.
   - 1 empty shell → do not migrate.
3. **ABI v5 (small, high-leverage):** add `in_position()`, `entry_price()`,
   `bars_since_entry()` — read-only position state the engine already has.
   This makes the 16 RR-exit translations *exact* instead of approximate and
   removes the ABI's only systematic expressiveness gap vs the rules DSL.
   Not a migration blocker (recipes work without it), but sequence it before
   promoting translated RR-exit strategies to live watching.
4. **Documentation repairs (cheap, do with the store ticket):**
   - Add `pending_order` to `STRATEGY_ABI.md`'s signal-map table (+ a golden
     script) — engine has supported it since AGT-607 (D6).
   - Note numeric-only boolean defaults and object-form select options (D7/D8).
   - Retire the SDK sections of `queries-service/src/rhai-strategy-prompt.md`
     in favor of generating the authoring prompt from `STRATEGY_ABI.md`, or
     at minimum sync it to v4 and fix its select-options example (D5/D7).
     One dialect, one document.

With 1–4 done, "app dialect" vs "wickd dialect" stops being a meaningful
distinction: one engine, one constructor, one ABI document, and a store whose
entries are all `wickd strategy validate`-clean v4 scripts.
