# D3 Live Market-Order Path Checklist (AGT-611)

> **Purpose:** verify the `wickd trade place --live` / `trade close --live` path
> **end-to-end**, and observe each guardrail (size cap, max-open cap, audit
> record-required) actually firing on a live order. The live guardrails were
> merged on LLM review alone (AGT-595/AGT-610) and had never been observed
> firing against a real broker — this checklist closes that trust gap.
>
> Run the automated steps on any machine (no account needed); run the
> **live-practice** steps against an OANDA **practice** account. Never run the
> live steps against a real-money (`--env live`) account.

## Legend

| Mark | Meaning |
|------|---------|
| **AUTO** | Automated — asserted by `cargo test`, no OANDA account required. |
| **LIVE** | Requires an OANDA **practice** account + `wickd login` on this machine. Run by a human. |

## Which ACs are automated vs. need a live practice account

| AC | What it proves | How it's verified | Coverage |
|----|----------------|-------------------|----------|
| AC1 | A documented, repeatable checklist exists | This file | — |
| AC2 | Dry-run (`no --live`) emits `submitted:false`, no network | `trade::tests::ac2_paper_place_submits_nothing_and_needs_no_network` | **AUTO** |
| AC3 | Oversize order rejected by size cap before any OANDA call | `trade::tests::ac3_oversize_rejected_pre_network_and_routes_to_validation` (+ `risk::tests::ac1_rejects_oversize_units`) | **AUTO** |
| AC4 | Market buy fills; audit row written **before** submit | ordering invariant: `trade::tests::ac4_audit_attempt_row_written_before_submit`; actual fill: step L2 | **AUTO** (ordering) + **LIVE** (fill) |
| AC5 | Second order breaching max-open is rejected | step L3 | **LIVE** |
| AC6 | `close` closes the position; short round-trip | steps L4–L5 | **LIVE** |
| AC7 | Unwritable audit store aborts a live attempt, no OANDA order | `trade::tests::ac7_unwritable_audit_store_aborts_before_any_oanda_call` | **AUTO** |

> **Credential gap (honest scope note):** AC4's actual fill, AC5's live
> max-open breach, and AC6's live close + short round-trip **cannot** be run in
> CI or in an agent worktree — they need a real OANDA practice account
> (`wickd login --env practice`) and place real practice orders. Everything that
> can be proven offline (AC2, AC3, AC7, and AC4's audit-before-submit ordering
> invariant) is covered by automated tests that drive the **real** `execute_place`
> path against a throwaway `$HOME`. The steps below marked **LIVE** are the ones
> a human must run against practice to close out AC4/AC5/AC6.

---

## Part A — Automated steps (AUTO)

Run from the repo root. These touch a throwaway `$HOME`, never your real
`~/.wickd/`.

- [ ] **A0. Build + full test suite + lint are clean**
  ```sh
  cargo build --workspace
  cargo test --workspace
  cargo clippy --workspace --all-targets -- -D warnings
  ```
  Expected: all green; clippy reports no warnings.

- [ ] **A1. AC2 — dry-run submits nothing, no network**
  ```sh
  cargo test -p wickd ac2_paper_place_submits_nothing_and_needs_no_network
  ```
  Expected: pass. Drives `execute_place(.., live=false)` with **no** credentials
  and asserts `submitted:false`, `mode:"paper"`, and a `not_submitted` audit row —
  the absence of any credential/network error proves no OANDA call was made.

- [ ] **A2. AC3 — oversize rejected before any OANDA call**
  ```sh
  cargo test -p wickd ac3_oversize_rejected_pre_network_and_routes_to_validation
  cargo test -p wickd ac1_rejects_oversize_units
  ```
  Expected: pass. The size cap is enforced by the pure `risk::enforce_pre_trade`
  guard (no I/O), and its `risk cap` message routes to `exit::VALIDATION`.

- [ ] **A3. AC4 (ordering) — audit row written BEFORE submit**
  ```sh
  cargo test -p wickd ac4_audit_attempt_row_written_before_submit
  ```
  Expected: pass. Drives the real live path against a writable temp `$HOME` with
  no creds: `record_required` writes the `attempt` row, then `client::resolve`
  fails — proving the ledger row is durable **before** the submit path is reached.

- [ ] **A4. AC7 — unwritable audit store aborts, no OANDA order**
  ```sh
  cargo test -p wickd ac7_unwritable_audit_store_aborts_before_any_oanda_call
  ```
  Expected: pass. With a read-only temp `.wickd`, `record_required` cannot write
  and the live attempt aborts with an audit-store error **before** credential
  resolution — proving `record_required` is pre-submit-fatal.

- [ ] **A5. Smoke-test the built binary's dry-run path (no account needed)**
  ```sh
  cargo build -p wickd
  ./target/debug/wickd trade place --instrument EUR_USD --units 1000
  ```
  Expected JSON (no `--live`): `"mode":"paper"`, `"submitted":false`, and the
  command returns without contacting OANDA or requiring credentials.

---

## Part B — Live practice steps (LIVE)

> **Preconditions**
> - An OANDA **practice** account.
> - `wickd login --env practice` completed on this machine (API key in the OS
>   keychain, account id in `~/.wickd/config.json`).
> - A risk config at `~/.wickd/risk.json`, e.g.:
>   ```json
>   { "max_position_size": 2000, "max_open_positions": 1 }
>   ```
>   (Small caps so both the size cap and the max-open cap are easy to trip.)
> - Market open for the instrument (EUR_USD) so a market order fills, not rests.
> - Inspect the audit ledger at any time with:
>   ```sh
>   wickd audit --limit 10
>   # or: sqlite3 ~/.wickd/audit.db 'SELECT id,ts,mode,action,outcome,detail FROM audit_log ORDER BY id DESC LIMIT 10'
>   ```

- [ ] **L0. Confirm arming semantics — `--env practice` alone does NOT submit**
  ```sh
  wickd trade place --instrument EUR_USD --units 1000 --env practice
  ```
  Expected: `"mode":"paper"`, `"submitted":false`. `--env` selects the account;
  only `--live` arms a real order.

- [ ] **L1. Baseline account state**
  ```sh
  wickd trade account --env practice
  wickd trade positions --env practice
  ```
  Expected: balance/NAV returns; note the current open-position count (ideally 0).

- [ ] **L2. AC4 (live) — a market buy within caps places, fills, and is audited**
  ```sh
  wickd trade place --instrument EUR_USD --units 1000 --live --yes --env practice
  ```
  Expected JSON: `"submitted":true`, `"outcome":"filled"` (`"filled":true`), a
  `trade_id`, `fill_id`, and `price`. Then:
  ```sh
  wickd trade positions --env practice   # EUR_USD long ~1000 units now open
  wickd audit --limit 5                  # newest-first
  ```
  Expected audit rows (newest first): a terminal `place/live/filled` row, and
  immediately below it the pre-submit `place/live/attempt` row. The **attempt**
  row's id is **lower** than the fill row's id → the audit write preceded the
  submit (AC4's "written before, not after"). Record both ids here: `attempt=___
  filled=___`.

- [ ] **L3. AC5 — a second order breaching max-open is rejected by the cap**
  With `max_open_positions: 1` and one position already open from L2:
  ```sh
  wickd trade place --instrument GBP_USD --units 1000 --live --yes --env practice
  echo "exit=$?"
  ```
  Expected: `ok:false` / non-zero exit (`exit::VALIDATION` = 2), message contains
  `risk cap` and `max-open-positions`. **No** GBP_USD position opens
  (`wickd trade positions`), and the audit ledger shows an `attempt` row followed
  by no fill for GBP_USD (the cap rejected it after the attempt row, before the
  submit). Confirm no order reached OANDA: `wickd trade orders --env practice`.

- [ ] **L4. AC6 — `close` closes the long from L2**
  ```sh
  wickd trade close --instrument EUR_USD --side long --live --yes --env practice
  wickd trade positions --env practice
  ```
  Expected: `"closed":true`, a `fill_id` and realized `pl`; EUR_USD no longer in
  positions. Audit shows `close/live/attempt` then `close/live/filled`.

- [ ] **L5. AC6 — short round-trip (open short, then close short)**
  ```sh
  wickd trade place --instrument EUR_USD --units -1000 --live --yes --env practice
  wickd trade positions --env practice          # EUR_USD short ~-1000 open
  wickd trade close --instrument EUR_USD --side short --live --yes --env practice
  wickd trade positions --env practice          # flat again
  ```
  Expected: the place returns `"side":"short"`, `submitted:true`, `filled`; the
  close returns `"closed":true`. Both produce attempt→terminal audit-row pairs.

- [ ] **L6. AC3 (live) — oversize order rejected before the broker sees it**
  With `max_position_size: 2000`:
  ```sh
  wickd trade place --instrument EUR_USD --units 5000 --live --yes --env practice
  echo "exit=$?"
  ```
  Expected: `ok:false` / exit 2, message contains `risk cap` and
  `exceeds the max position size`. No EUR_USD position opens; no order reaches
  OANDA (`wickd trade orders`). (The offline test A2 already proves the rejection
  is pre-network; this confirms it on the live path too.)

- [ ] **L7. AC7 (live, optional) — unwritable audit store aborts a live order**
  > Optional: the offline test A4 already proves this against a temp store.
  > If reproducing live, use a **throwaway** login in a temp HOME — do **not**
  > chmod your real `~/.wickd/`.
  ```sh
  chmod 0500 ~/.wickd          # make the audit dir unwritable (temp HOME only!)
  wickd trade place --instrument EUR_USD --units 1000 --live --yes --env practice
  echo "exit=$?"
  chmod 0700 ~/.wickd          # restore
  ```
  Expected: non-zero exit, error mentions the audit store (not credentials); no
  order reaches OANDA (`wickd trade orders`).

- [ ] **L8. Restore caps / clean up**
  Reset `~/.wickd/risk.json` to your normal caps (or remove it to disarm caps),
  and confirm the practice account is flat (`wickd trade positions`).

---

## Part C — Limit/Stop entry orders (AGT-612)

> Extends this checklist to the **limit/stop entry** path added in AGT-612. The
> classifier's four outcomes (filled / rejected-or-cancelled / rested-pending /
> partial), the per-instrument price-precision fix, and the request-body shaping
> are all proven **AUTO** by unit tests; the one thing that genuinely needs a
> broker is a real **resting** round-trip (AC6), which is **LIVE**.

### Which AGT-612 ACs are automated vs. need a live practice account

| AC | What it proves | How it's verified | Coverage |
|----|----------------|-------------------|----------|
| AC1 | Limit/Stop go through the SINGLE guarded `execute_place` path (caps + audit apply identically) | `trade::tests::limit_without_price_is_a_validation_error` (+ the shared arming/caps/audit tests in Parts A/B run for every kind) | **AUTO** |
| AC2 | Limit/Stop POST `/orders` with default `GTC` + optional `gtdTime`/`priceBound`/`triggerCondition`; price precision fixed | `types::tests::{limit_entry_defaults_gtc_and_formats_price, stop_entry_carries_optional_fields, price_precision_is_per_instrument}`; `endpoints::tests::test_place_entry_order_limit_rests` | **AUTO** |
| AC3 | Classifier distinguishes filled / rejected-or-cancelled / rested / partial using `orderRejectTransaction` | `trade::tests::classify_entry_*` (5 tests) + `endpoints::tests::test_place_entry_order_hard_reject` | **AUTO** |
| AC4 | Every classification path writes the audit row with the TRUE outcome before returning | ordering proven by `trade::tests::ac4_audit_attempt_row_written_before_submit`; per-outcome audit writes are inline in `execute_place` | **AUTO** (ordering) |
| AC5 | A rested order marks its pending signal consumed so `approve.rs` can't re-approve | `approve::tests::resting_order_consumes_signal_blocking_a_second_approve_attempt` | **AUTO** |
| AC6 | A practice limit/stop order round-trip rests + classifies correctly | steps L9–L11 | **LIVE** |

> **Credential gap (honest scope note):** AC6 — placing a *real* resting limit
> (or stop) order against a practice account, confirming it shows in
> `wickd trade orders`, and observing the `resting` classification + audit row —
> **cannot** run in CI or an agent worktree. It needs `wickd login --env practice`
> and places a real (resting) practice order. Everything else in AGT-612 is
> covered by the AUTO tests above (synthetic OANDA responses drive each of the
> four classifier outcomes; the request builder + precision are asserted on the
> serialized body). The steps below marked **LIVE** are what a human runs to
> close out AC6.

- [ ] **L9. AC6 (live) — a resting LIMIT entry places, rests, and is classified**
  Pick a limit price well away from the market so it does NOT fill immediately
  (e.g. a buy limit ~50 pips below the current bid). With market open:
  ```sh
  wickd trade place --instrument EUR_USD --units 1000 \
    --type limit --price <far-below-market> --tif gtc --live --yes --env practice
  echo "exit=$?"
  ```
  Expected JSON: `"submitted":true`, `"outcome":"resting"` (`"filled":false`),
  `"type":"limit"`, an `order_id`, and `ok:true`. Then:
  ```sh
  wickd trade orders --env practice     # the LIMIT order is listed, state PENDING
  wickd audit --limit 5                 # newest-first
  ```
  Expected audit rows (newest first): a terminal `place/live/resting` row whose
  detail names `type=limit` and the `order_id`, and immediately below it the
  pre-submit `place/live/attempt` row (attempt id lower than the resting row's).
  Record: `attempt=___ resting=___ order_id=___`.

- [ ] **L10. AC6 (live) — cancel the resting order (cleanup)**
  Cancel the pending order from L9 via the OANDA web platform (or leave it to
  expire), then confirm it is gone:
  ```sh
  wickd trade orders --env practice     # the L9 LIMIT order no longer PENDING
  ```

- [ ] **L11. AC2 (live) — precision guard: a mis-precisioned price is handled**
  Optional. Submit a limit price with too many decimals for the instrument and
  confirm the client formats it to the instrument's precision (so OANDA does not
  reject with `PRICE_PRECISION_EXCEEDED`):
  ```sh
  wickd trade place --instrument EUR_USD --units 1000 \
    --type limit --price 1.0755123 --tif gtc --live --yes --env practice
  ```
  Expected: the order rests (or fills) — it is NOT rejected for price precision;
  the echoed `price` is 5-dp. Cancel it afterwards as in L10.

---

## Sign-Off

| Role | Name | Date | Env | AUTO steps | LIVE steps |
|------|------|------|-----|-----------|-----------|
| Engineer | | | practice | [ ] | [ ] |
| Reviewer | | | practice | [ ] | [ ] |

## Notes

_Record ids, fills, and any surprises here:_

```
[Date] [Step] [Observation]
```
