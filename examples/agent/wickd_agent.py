#!/usr/bin/env python3
"""Reference local-agent entrypoint that drives `wickd` over its JSON I/O.

This is the *brain* the wickd architecture brief calls for: a local Claude agent
that SHELLS OUT to the `wickd` binary, reads the JSON it prints on stdout,
reasons over it, and issues the next `wickd` command. It is the data -> reason
-> orders loop, made runnable.

Design invariants (match AGT-564 acceptance criteria):
  * The agent WRAPS the `wickd` binary as a subprocess. No trading/strategy model
    is embedded here and none is embedded in the Rust CLI — wickd stays the hands
    and eyes, the agent is the brain. (AC4)
  * Reasoning is delegated to a local Claude agent step via the `claude` CLI in
    headless `-p` (print) mode. That keeps the model/tooling layer thin and
    obviously a reference example; swap it for the Anthropic SDK or a local LLM
    without touching the wickd-wrapping loop.
  * PAPER BY DEFAULT. This example NEVER passes `--live` to `wickd trade place`.
    Per the wickd arming convention (AGT-592), an order is paper/dry-run unless
    `--live` is explicitly armed; this harness emits the would-be order as JSON
    and stops there. Arming real money is deliberately out of scope for the
    reference example.

The loop:
  1. DATA   — `wickd strategy run <strategy> <instrument> ...` (+ optional
              `wickd trade account`) → parse JSON.
  2. REASON — hand that JSON to a Claude agent step (`claude -p`) and get back a
              strict-JSON decision: place or hold.
  3. ORDERS — on a `place` decision, run `wickd trade place ...` (paper) and
              surface the would-be order JSON. Loop.

Quick start:
  examples/agent/wickd_agent.py --explain               # offline: print the plan + the exact wickd commands
  examples/agent/wickd_agent.py --instrument EUR_USD    # one real data->reason->orders pass (needs wickd creds + claude)
  examples/agent/wickd_agent.py --no-llm                # skip the model; use the built-in heuristic brain (still paper)

Runtime requirements (only for a live data/reason pass — `--explain` needs none):
  * `wickd` on PATH (or pass --wickd / set WICKD_BIN), with OANDA creds in its vault.
  * `claude` on PATH for the reason step, unless --no-llm is given.

JSON contract this harness relies on (stable, see crates/wickd/src/output.rs):
  * Every wickd command prints exactly one JSON object on stdout and exits 0 on success.
  * Errors print {"error":{"code","message"}} and exit non-zero (2 auth, 3 oanda,
    4 validation). This harness raises WickdError carrying that envelope.
  * `strategy run` -> {strategy, instrument, granularity, candles, signals:[...],
                       summary:{buy,sell,close,hold}}
  * `trade place` (paper) -> {ok, mode:"paper", submitted:false, environment,
                              instrument, units, side, sl, tp}
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from typing import Any, Optional


class WickdError(RuntimeError):
    """A `wickd` invocation failed; carries the parsed JSON error envelope."""

    def __init__(self, argv: list[str], code: int, envelope: Optional[dict]):
        self.argv = argv
        self.code = code
        self.envelope = envelope or {}
        err = self.envelope.get("error", {})
        msg = err.get("message") or f"wickd exited {code}"
        super().__init__(f"{' '.join(argv)} -> [{err.get('code', code)}] {msg}")


# --------------------------------------------------------------------------- #
# wickd: the binary the agent wraps. Every call shells out and parses JSON.    #
# --------------------------------------------------------------------------- #
class Wickd:
    def __init__(self, binary: str):
        self.binary = binary

    def _argv(self, args: list[str]) -> list[str]:
        return [self.binary, *args]

    def run(self, args: list[str]) -> dict[str, Any]:
        """Run `wickd <args>`, returning the parsed JSON object on stdout.

        Raises WickdError on a non-zero exit (parsing the error envelope) or on
        output that is not a single JSON object.
        """
        argv = self._argv(args)
        proc = subprocess.run(argv, capture_output=True, text=True)
        stdout = proc.stdout.strip()
        try:
            parsed = json.loads(stdout) if stdout else None
        except json.JSONDecodeError:
            parsed = None
        if proc.returncode != 0:
            raise WickdError(argv, proc.returncode, parsed if isinstance(parsed, dict) else None)
        if not isinstance(parsed, dict):
            raise WickdError(argv, proc.returncode, {
                "error": {"code": "bad_output",
                          "message": f"expected one JSON object, got: {stdout[:200]!r}"}})
        return parsed

    # --- the three subcommands this reference loop drives ------------------ #
    def strategy_run(self, strategy: str, instrument: str, granularity: str,
                     count: int, env: str) -> dict[str, Any]:
        return self.run(["strategy", "run", strategy, instrument,
                         "--granularity", granularity, "--count", str(count),
                         "--env", env])

    def trade_account(self, env: str) -> dict[str, Any]:
        return self.run(["trade", "account", "--env", env])

    def trade_place_paper(self, instrument: str, units: int,
                          sl: Optional[str], tp: Optional[str], env: str) -> dict[str, Any]:
        """Place an order in PAPER mode. Note: `--live` is intentionally never
        passed here, so wickd computes the would-be order and never contacts
        OANDA's submission endpoints."""
        args = ["trade", "place", "--instrument", instrument,
                "--units", str(units), "--env", env]
        if sl:
            args += ["--sl", sl]
        if tp:
            args += ["--tp", tp]
        return self.run(args)


# --------------------------------------------------------------------------- #
# REASON: the brain. Default path delegates to a local Claude agent (claude -p)#
# --------------------------------------------------------------------------- #
DECISION_SCHEMA = (
    '{"action":"place"|"hold","units":<int, negative=short>,'
    '"sl":<string price or null>,"tp":<string price or null>,'
    '"rationale":<short string>}'
)


# OANDA account fields that identify the account (routing keys / user IDs).
# These have no reasoning value for a sizing decision and must NOT leave the
# local machine — `claude -p` makes an outbound call to Anthropic's API, so any
# account JSON fed into the prompt is exfiltrated. Strip them first.
_ACCOUNT_IDENTITY_FIELDS = ("id", "accountID", "createdByUserID", "alias", "mt4AccountID")


def _sanitize_account(account: Optional[dict[str, Any]]) -> Optional[dict[str, Any]]:
    """Drop account-identifying fields before the summary enters the prompt.
    Balance, NAV, margin, and open-trade counts — what actually informs sizing —
    are retained; the account identifier is not."""
    if not isinstance(account, dict):
        return account
    return {k: v for k, v in account.items() if k not in _ACCOUNT_IDENTITY_FIELDS}


def build_prompt(context: dict[str, Any], instrument: str) -> str:
    """Build the reasoning prompt fed to `claude -p`. The model is the strategy
    brain; wickd already supplied the data as JSON."""
    return (
        "You are the trading brain sitting on top of the `wickd` CLI. wickd has "
        "already fetched market data and run a strategy; its JSON is below. "
        "Decide the next action for a PAPER (dry-run) order — no real money is at "
        "risk. Size conservatively.\n\n"
        f"Instrument: {instrument}\n"
        f"wickd strategy output:\n{json.dumps(context.get('strategy'), indent=2)}\n\n"
        f"Account summary (may be null):\n{json.dumps(_sanitize_account(context.get('account')), indent=2)}\n\n"
        "Respond with ONLY a JSON object, no prose, matching exactly:\n"
        f"{DECISION_SCHEMA}\n"
        "Use \"hold\" with units 0 if there is no clear, recent signal."
    )


def reason_with_claude(context: dict[str, Any], instrument: str,
                       claude_bin: str, model: Optional[str]) -> dict[str, Any]:
    """Delegate the decision to a local Claude agent via headless `claude -p`."""
    prompt = build_prompt(context, instrument)
    argv = [claude_bin, "-p", prompt, "--output-format", "json"]
    if model:
        argv += ["--model", model]
    proc = subprocess.run(argv, capture_output=True, text=True)
    if proc.returncode != 0:
        raise RuntimeError(f"claude -p failed ({proc.returncode}): {proc.stderr.strip()[:300]}")
    # `--output-format json` wraps the turn; the model's text is in `.result`.
    try:
        envelope = json.loads(proc.stdout)
        text = envelope.get("result", proc.stdout) if isinstance(envelope, dict) else proc.stdout
    except json.JSONDecodeError:
        text = proc.stdout
    return _extract_decision(text)


def _extract_decision(text: str) -> dict[str, Any]:
    """Pull the first JSON object out of the model's text and validate it."""
    start, end = text.find("{"), text.rfind("}")
    if start == -1 or end == -1 or end < start:
        raise ValueError(f"no JSON object in model output: {text[:200]!r}")
    decision = json.loads(text[start:end + 1])
    action = decision.get("action")
    if action not in ("place", "hold"):
        raise ValueError(f"decision.action must be 'place' or 'hold', got {action!r}")
    return decision


def reason_with_heuristic(context: dict[str, Any]) -> dict[str, Any]:
    """Offline fallback brain (--no-llm): act on the most recent strategy signal.

    Deliberately trivial — the point of the example is the wraps-the-binary loop,
    not this heuristic. It lets the harness run end to end without `claude`."""
    signals = (context.get("strategy") or {}).get("signals") or []
    last = signals[-1] if signals else None
    if not last or last.get("signal") not in ("buy", "sell"):
        return {"action": "hold", "units": 0, "sl": None, "tp": None,
                "rationale": "no recent buy/sell signal"}
    units = 1000 if last["signal"] == "buy" else -1000
    return {"action": "place", "units": units, "sl": None, "tp": None,
            "rationale": f"acting on latest {last['signal']} signal at {last.get('time')}"}


# --------------------------------------------------------------------------- #
# The loop                                                                    #
# --------------------------------------------------------------------------- #
def explain(args: argparse.Namespace) -> int:
    """Offline dry path: print the loop plan and the exact wickd commands this
    harness would run. Touches nothing — safe to run anywhere (smoke test)."""
    plan = {
        "loop": "data -> reason -> orders",
        "wraps_binary": args.wickd,
        "model_step": "heuristic (--no-llm)" if args.no_llm else f"claude -p (model={args.model or 'default'})",
        "paper_only": True,
        "never_passes": "--live",
        "commands": {
            "1_data": [args.wickd, "strategy", "run", args.strategy, args.instrument,
                       "--granularity", args.granularity, "--count", str(args.count),
                       "--env", args.env],
            "1_data_account": [args.wickd, "trade", "account", "--env", args.env],
            "3_orders_example": [args.wickd, "trade", "place", "--instrument", args.instrument,
                                 "--units", "<from decision>", "--env", args.env,
                                 "# NOTE: no --live -> paper/dry-run"],
        },
        "decision_schema": DECISION_SCHEMA,
    }
    print(json.dumps(plan, indent=2))
    return 0


def run_once(args: argparse.Namespace, wickd: Wickd) -> int:
    # 1. DATA -------------------------------------------------------------- #
    log(args, f"[data] wickd strategy run {args.strategy} {args.instrument}")
    context: dict[str, Any] = {
        "strategy": wickd.strategy_run(args.strategy, args.instrument,
                                       args.granularity, args.count, args.env),
        "account": None,
    }
    try:
        context["account"] = wickd.trade_account(args.env)
    except WickdError as e:
        # Account context is optional (paper reasoning still works without it).
        log(args, f"[data] account unavailable ({e}); continuing without it")

    summary = (context["strategy"] or {}).get("summary")
    log(args, f"[data] signals summary: {json.dumps(summary)}")

    # 2. REASON ------------------------------------------------------------ #
    if args.no_llm:
        log(args, "[reason] heuristic brain (--no-llm)")
        decision = reason_with_heuristic(context)
    else:
        log(args, f"[reason] claude -p (model={args.model or 'default'})")
        decision = reason_with_claude(context, args.instrument, args.claude, args.model)
    log(args, f"[reason] decision: {json.dumps(decision)}")

    # 3. ORDERS ------------------------------------------------------------ #
    if decision["action"] == "hold":
        result = {"action": "hold", "rationale": decision.get("rationale")}
        log(args, "[orders] hold — no order placed")
    else:
        units = int(decision["units"])
        log(args, f"[orders] wickd trade place (PAPER) units={units}")
        result = wickd.trade_place_paper(
            args.instrument, units, decision.get("sl"), decision.get("tp"), args.env)
        # Defense in depth: the harness only calls the paper path, so wickd must
        # report paper. If it ever didn't, fail loudly rather than mask it.
        if result.get("mode") != "paper" or result.get("submitted") is not False:
            raise RuntimeError(f"expected a paper order, got: {json.dumps(result)}")

    print(json.dumps({"decision": decision, "order": result}, indent=2))
    return 0


def log(args: argparse.Namespace, msg: str) -> None:
    if not args.quiet:
        print(msg, file=sys.stderr)


def parse_args(argv: Optional[list[str]] = None) -> argparse.Namespace:
    p = argparse.ArgumentParser(
        prog="wickd_agent.py",
        description="Reference local Claude agent that drives wickd (data -> reason -> orders). Paper by default.",
    )
    p.add_argument("--instrument", default="EUR_USD", help="instrument to trade (default: EUR_USD)")
    p.add_argument("--strategy", default="ma-crossover", help="strategy for `wickd strategy run` (default: ma-crossover)")
    p.add_argument("--granularity", default="H1", help="candle granularity (default: H1)")
    p.add_argument("--count", type=int, default=200, help="recent candle count (default: 200)")
    p.add_argument("--env", default="practice", help="OANDA env for wickd (default: practice)")
    p.add_argument("--iterations", type=int, default=1, help="number of data->reason->orders passes (default: 1)")
    p.add_argument("--wickd", default=os.environ.get("WICKD_BIN", "wickd"),
                   help="wickd binary to wrap (default: $WICKD_BIN or `wickd` on PATH)")
    p.add_argument("--claude", default=os.environ.get("CLAUDE_BIN", "claude"),
                   help="claude CLI for the reason step (default: $CLAUDE_BIN or `claude` on PATH)")
    p.add_argument("--model", default=None, help="model id passed to `claude -p` (default: claude's default)")
    p.add_argument("--no-llm", action="store_true",
                   help="skip the Claude reason step; use the built-in heuristic brain (still paper)")
    p.add_argument("--explain", action="store_true",
                   help="print the loop plan + exact wickd commands and exit (offline; runs nothing)")
    p.add_argument("--quiet", action="store_true", help="suppress the per-step log on stderr")
    return p.parse_args(argv)


def main(argv: Optional[list[str]] = None) -> int:
    args = parse_args(argv)

    if args.explain:
        return explain(args)

    if shutil.which(args.wickd) is None and not os.path.exists(args.wickd):
        print(json.dumps({"error": {"code": "wickd_not_found",
                                    "message": f"wickd binary not found: {args.wickd!r} "
                                               "(set --wickd / WICKD_BIN, or run `cargo build -p wickd`). "
                                               "Try --explain for an offline dry run."}}),
              file=sys.stderr)
        return 4
    if not args.no_llm and shutil.which(args.claude) is None and not os.path.exists(args.claude):
        print(json.dumps({"error": {"code": "claude_not_found",
                                    "message": f"claude CLI not found: {args.claude!r} "
                                               "(set --claude / CLAUDE_BIN, or pass --no-llm)."}}),
              file=sys.stderr)
        return 4

    wickd = Wickd(args.wickd)
    try:
        for i in range(args.iterations):
            if args.iterations > 1:
                log(args, f"=== pass {i + 1}/{args.iterations} ===")
            run_once(args, wickd)
        return 0
    except WickdError as e:
        print(json.dumps({"error": {"code": "wickd_failed", "message": str(e)}}), file=sys.stderr)
        return 3
    except (RuntimeError, ValueError, json.JSONDecodeError) as e:
        print(json.dumps({"error": {"code": "agent_failed", "message": str(e)}}), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
