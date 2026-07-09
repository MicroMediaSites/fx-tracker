# Reference agent entrypoint for `wickd`

`wickd_agent.py` is a runnable reference for the wickd thesis: **data in, orders
out — your agent supplies the strategy.** A local Claude agent SHELLS OUT to the
`wickd` binary, reads the JSON it prints, reasons over it, and issues the next
`wickd` command. The agent wraps the binary; no model is embedded in the Rust CLI.

```
  ┌─────────────────────────────┐
  │  wickd_agent.py  (the brain) │
  │  data → reason → orders      │
  └───────▲──────────────┬───────┘
   JSON   │              │ commands
  ┌───────┴──────────────▼───────┐
  │  wickd  (the hands and eyes) │   ← wrapped as a subprocess
  └──────────────────────────────┘
```

## The loop

1. **DATA** — `wickd strategy run <strategy> <instrument> …` (+ optional
   `wickd trade account`) → parse the JSON.
2. **REASON** — hand that JSON to a local Claude agent step (`claude -p`, headless)
   and get back a strict-JSON decision (`place` or `hold`). Swappable for the
   Anthropic SDK or a local LLM without touching the wickd-wrapping loop.
3. **ORDERS** — on a `place` decision, run `wickd trade place …` and surface the
   would-be order. Loop.

## Paper by default — the safety note

This example **never passes `--live`** to `wickd trade place`. Per the wickd arming
convention, an order is paper/dry-run unless `--live` is explicitly armed; the
harness emits the would-be order JSON (`"mode":"paper","submitted":false`) and
stops there. It also asserts the returned order is paper and fails loudly otherwise.
Arming real money is deliberately out of scope for the reference example.

## Run it

```sh
# Offline — print the loop plan and the exact wickd commands, run nothing:
examples/agent/wickd_agent.py --explain

# One real data→reason→orders pass (needs wickd creds in its vault + claude on PATH):
examples/agent/wickd_agent.py --instrument EUR_USD --strategy ma-crossover

# Skip the model; use the built-in heuristic brain (still paper, no claude needed):
examples/agent/wickd_agent.py --no-llm --instrument EUR_USD

# Loop several passes:
examples/agent/wickd_agent.py --iterations 5
```

Key flags: `--instrument`, `--strategy`, `--granularity`, `--count`, `--env`,
`--iterations`, `--wickd`/`$WICKD_BIN` (the binary to wrap), `--claude`/`$CLAUDE_BIN`,
`--model`, `--no-llm`, `--explain`, `--quiet`. See `--help`.

## Requirements

- `wickd` on `PATH` (or `--wickd` / `$WICKD_BIN`), with OANDA creds in its vault
  (`wickd login`). Build it with `cargo build -p wickd`. Not needed for `--explain`.
- `claude` on `PATH` for the reason step, unless `--no-llm`. Not needed for `--explain`.
- Python 3 (standard library only).

## The JSON contract it relies on

Every `wickd` command prints exactly one JSON object on stdout and exits 0 on
success; errors print `{"error":{"code","message"}}` and exit non-zero (2 auth,
3 oanda, 4 validation). The harness raises on the error envelope. See
`crates/wickd/src/output.rs` for the canonical contract.
