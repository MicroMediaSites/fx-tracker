# Agent Engineering Handbook

This handbook defines the shared engineering practices that ALL domain agents must follow,
regardless of which domain they own. It is loaded into every subagent's context alongside
their domain-specific knowledge base.

This handbook is law. The review bot enforces it.

---

## 1. Task Lifecycle

Every task follows a structured lifecycle. The steps differ by task type. Skipping steps
is never acceptable — the research step exists because agents that skip it introduce bugs.

### 1.1 Bug Fixes

1. **Research** — Read the full BUGS.md for the affected domain(s). Search for prior
   incidents with similar symptoms, affected components, or root causes. Summarize what
   you found (or explicitly state "No prior bugs match this symptom"). You must PROVE you
   did the reading before proceeding.

2. **Diagnose** — Identify the root cause. Document your diagnostic reasoning: what you
   checked, what you ruled out, and why you believe this is the cause. Read the actual
   code involved — do not guess based on file names.

3. **Plan** — Propose a fix. If multiple approaches exist, document alternatives and why
   you chose this one. Check ARCHITECTURE.md for invariants the fix must respect.

4. **Implement** — Write the fix with appropriate tests. Run `npm run test:be` for Rust
   changes. Verify the fix doesn't violate any invariants listed in ARCHITECTURE.md.

5. **Document** — Answer ALL mandatory documentation questions (Section 2).

6. **Self-review** — Re-read your changes against the domain's CONVENTIONS.md and
   ARCHITECTURE.md. Are you introducing anything that contradicts existing patterns?

### 1.2 Feature Work

1. **Research** — Read ARCHITECTURE.md and INTERFACES.md for ALL affected domains.
   Understand the existing design before extending it. If the feature involves a
   cross-cutting concern (indicators, strategy types, schema), read the primary owner's
   knowledge base AND all affected consumers' INTERFACES.md files.

2. **Plan** — Propose the design. Document key decisions and alternatives considered.
   If the feature spans multiple domains, identify the interface contracts that need to
   change and propose them BEFORE implementation. The orchestrator coordinates this.

3. **Implement** — Build the feature with appropriate tests. Follow the vertical pattern:
   backend changes first, then frontend, within each domain.

4. **Document** — Answer ALL mandatory documentation questions (Section 2).

5. **Self-review** — Same as bug fix self-review.

### 1.3 Refactors

1. **Research** — Read ARCHITECTURE.md and CONVENTIONS.md. Understand WHY things are the
   way they are before changing them. There may be documented reasons for patterns that
   look wrong. Check BUGS.md — a pattern you want to "clean up" may exist specifically
   to prevent a past bug.

2. **Plan** — Document what you're changing and why. Identify any interface impacts.
   Follow the Safe Refactoring Protocol in `docs/engineering-principles.md` Section 5.5.

3. **Implement** — Make the changes. Verify after each step — do not batch changes.

4. **Document** — Answer ALL mandatory documentation questions (Section 2).

5. **Self-review** — Same as above.

---

## 2. Mandatory Documentation Questions

After completing implementation, every agent must answer ALL of these questions. The
answers determine which knowledge base files need updating. "No" is a valid answer but
must be justified.

1. **Did any public interface change?** (function signatures, trait definitions, API
   contracts, shared types, Tauri commands, Zero queries/mutators)
   → If yes: update INTERFACES.md in this domain AND list which other domains are affected.

2. **Did you make a decision where multiple reasonable approaches existed?**
   → If yes: document in ARCHITECTURE.md with decision, alternatives, and rationale.

3. **Did you discover something non-obvious that would surprise someone seeing this code
   for the first time?**
   → If yes: add to CONVENTIONS.md.

4. **Did you fix a bug?**
   → If yes: BUGS.md entry is MANDATORY with ALL fields (see Section 3 for quality standards).

5. **Did you create new files or directories?**
   → If yes: verify they fall within this domain's OWNERSHIP.md globs. If they don't,
   update OWNERSHIP.md.

6. **Did you touch files that belong to another domain?**
   → If yes: flag this for the orchestrator. Cross-domain changes need review from both
   domain agents.

---

## 3. Documentation Quality Standards

### 3.1 BUGS.md Entries

**A good BUGS.md entry** (real example from this codebase):

```
## 2026-02-15 Missing SL in backtest results and missing Ichimoku displacement
- **Symptom**: Backtest trades were showing no stop loss values, and Ichimoku cloud
  was not displaced correctly.
- **Root Cause**: The stop loss from the strategy was not being propagated through the
  `ExtendedSignal` into `SimulatedTrade`, and the Ichimoku indicator was not applying
  the displacement parameter.
- **Fix**: Commit `b46423b` fixed both issues — SL is now captured in the extended
  signal path and the Ichimoku displacement is applied during indicator calculation.
- **Prevention**: When adding new fields to `ExtendedSignal` or `SimulatedTrade`, always
  verify the field flows through the entire chain: `RulesEngine` -> `RulesSignal` ->
  `ExtendedSignal` -> `SimulatedTrade`. Add a test that asserts the value appears in
  the final trade output.
```

Why this is good: A future agent encountering a similar issue (new field not appearing
in results) can search BUGS.md, find this entry, and immediately know to check the full
signal chain. The prevention pattern is actionable and specific.

**A bad BUGS.md entry:**

```
## Fixed missing SL bug
- **Symptom**: SL was missing
- **Root Cause**: It wasn't being passed through
- **Fix**: Added the field
- **Prevention**: Test your changes
```

Why this is bad: "It wasn't being passed through" doesn't tell a future agent WHERE in
the chain it broke. "Test your changes" is not a pattern — it's a platitude. A future
agent learns nothing from this entry.

### 3.2 ARCHITECTURE.md Decisions

**A good architecture decision** (real example from backtest-core):

```
### SL Priority Over TP on Same-Bar Breach

When both stop loss and take profit are breached on the same candle, stop loss wins.
This is a conservative assumption because intra-bar order of price movement is unknown.
Assuming the worst case protects against overfitting to favorable bar sequences.
```

Why this is good: It states the decision, the reasoning, and the consequence of doing
it differently (overfitting). A future agent who thinks "let's make TP win for better
results" will read this and understand why that would be wrong.

**A bad architecture decision:**

```
### Order Priority

SL is checked before TP. This is how backtesting engines typically work.
```

Why this is bad: "This is how backtesting engines typically work" is an appeal to
convention, not a reason. It doesn't explain the specific tradeoff (overfitting risk)
or why a different choice would be harmful in this codebase.

### 3.3 CONVENTIONS.md Entries

**A good convention** (real example from oanda-trading):

```
### Never use current_units for trade direction

// WRONG - current_units is 0 for closed trades
let is_long = trade.current_units.parse::<Decimal>().unwrap() > Decimal::ZERO;

// RIGHT - initial_units preserves the original direction
let is_long = trade.initial_units.parse::<Decimal>().unwrap() > Decimal::ZERO;
```

Why this is good: It shows the specific trap (closed trades have zero current_units),
gives both wrong and right code, and a future agent can pattern-match against this
when writing trade direction logic.

**A bad convention:**

```
### Use the right units field

Make sure to use the correct units field when checking trade direction.
```

Why this is bad: It doesn't say WHICH field is correct, WHY the wrong one fails, or
WHEN the failure occurs. A future agent has no actionable guidance.

---

## 4. Cross-Domain Coordination Rules

### 4.1 When Cross-Domain Work Is Detected

The orchestrator detects cross-domain work by checking if a task touches files in
multiple domains' OWNERSHIP.md patterns. When this happens:

1. **Interface contracts first** — Before any implementation, the orchestrator identifies
   which INTERFACES.md files are affected and proposes the contract changes to all
   affected domain agents for review.

2. **Primary domain implements first** — The domain that owns the interface being changed
   goes first. Consuming domains adapt after.

3. **Both domains update docs** — Each affected domain must update its own knowledge base
   files. The primary domain updates INTERFACES.md with the new contract; consuming
   domains update INTERFACES.md to reflect the new dependency.

### 4.2 Cross-Cutting Concerns

Some code is consumed by many domains. These have a designated primary owner:

| Concern | Primary Owner | Consumers | Coordination Rule |
|---------|--------------|-----------|-------------------|
| Indicators (computation) | `indicators` | backtest-core, strategy-monitor, charting, ai-analysis | Any change to indicator output names, types, or computation must be reviewed by all consumers |
| Strategy types (`shared/src/lib.rs`) | `backtest-core` | strategy-monitor, mcp-server, data-infrastructure, charting | Changes to `StrategyDefinition`, `Trigger`, `DataSource` require multi-domain review |
| Zero schema (`shared/schema.ts`) | `data-infrastructure` | Every domain that reads/writes data | Schema changes follow the 3-location sync checklist in data-infrastructure CONVENTIONS.md |
| Feature gating (`packages/content/`) | `membership-payments` | Every domain with gated features | New features get tier assignment from membership-payments, then gating UI from implementing domain |
| Candle alignment settings | `oanda-trading` | backtest-core, strategy-monitor, charting | dailyAlignment=3, alignmentTimezone=UTC — NEVER change without coordinating all consumers |

### 4.3 Shared File Protocol

When a task requires modifying a file that appears in multiple domains' OWNERSHIP.md:

1. Check which domain is the **primary owner** (listed in OWNERSHIP.md shared files table)
2. Make the change via the primary owner's agent
3. Notify all consuming domains so they can update their INTERFACES.md if needed
4. If ownership is ambiguous, escalate to the user

### 4.4 Order of Operations for Multi-Domain Features

Features are built vertically (backend → frontend within each domain). When multiple
domains are involved:

1. Data layer changes first (`data-infrastructure` — migrations, schema, queries)
2. Backend logic second (the domain that owns the business logic)
3. Frontend last (the domain that owns the UI)
4. Feature gating (`membership-payments`) can be done in parallel with step 3

---

## 5. What Agents Must NEVER Do

### 5.1 Ownership Violations

- **Never modify code in another domain's ownership boundary without flagging it.** If
  your task requires changing a file owned by another domain, report this to the
  orchestrator. Cross-domain changes need review from both domain agents.

- **Never add files outside your domain's OWNERSHIP.md patterns without updating
  OWNERSHIP.md.** Unowned files are flagged by the review bot.

### 5.2 Research Violations

- **Never skip the research step.** Even if the fix seems obvious, read BUGS.md first.
  The "obvious" fix may have been tried before and reverted for a reason documented there.

- **Never assume a past BUGS.md entry is still accurate without checking the current
  code.** Fixes may have been reverted or the code may have changed.

### 5.3 Documentation Violations

- **Never update ARCHITECTURE.md to describe how you WISH the code worked.** Only
  document what IS, plus explicit notes about desired future state labeled as such.

- **Never write documentation that restates the code.** Documentation should capture
  the WHY and the NON-OBVIOUS, not narrate what the code does.

- **Never skip the mandatory documentation questions.** Every completed task must answer
  all 6 questions in Section 2. "No" is valid; silence is not.

### 5.4 Code Safety Violations

- **Never use f64 for financial values.** Use `rust_decimal::Decimal` in Rust, string
  representation in JSON/TypeScript. This is an invariant across ALL domains.

- **Never add database migrations outside `queries-service/src/migrate.ts`.** The Rust
  backend (`db.rs`) only runs queries. CI will fail if migrations are added elsewhere.

- **Never embed API keys or secrets in code.** Use environment variables or the
  credential vault. Production AI calls go through the queries-service proxy.

- **Never touch `.env` files.** These contain real credentials and are outside all
  domain boundaries.

### 5.5 Git & Deployment

- Commits and pull requests are allowed freely.
- Do NOT run `railway` CLI commands without asking the user first.
- Do NOT use `--force` flags on any git command without asking.

### 5.6 Process Violations

- **Never run `npm run tauri build`, `./build-prod.sh`, or `./build-staging.sh`.** Build
  commands are never executed by agents.

- **Never leave background processes running.** If you start a dev server to test
  something, kill it immediately after checking output. Clean up ports 1420/4848.

---

## 6. Testing Requirements

### 6.1 When Tests Are Required

- **Bug fixes**: Always. The test should reproduce the bug and verify the fix.
- **New features**: Always for backend logic. Frontend tests are recommended for
  complex interactions but not mandatory for every component.
- **Refactors**: Always. Existing tests must continue to pass. Add tests if the
  refactored code path wasn't previously covered.

### 6.2 Running Tests

```bash
# Rust backend tests
npm run test:be

# Rust backend with coverage
npm run test:be:cov

# Frontend tests
npx vitest run

# All tests must pass before considering work complete
```

### 6.3 Test Patterns by Domain

Each domain's CONVENTIONS.md documents domain-specific testing patterns. Common patterns:

- **Rust endpoints**: Use `wiremock::MockServer` to mock HTTP responses
- **Rust models**: Test `From` implementations with valid and invalid input
- **Rust indicators**: Test warmup behavior, edge cases (all same values, single candle)
- **Frontend hooks**: Test with Vitest and React Testing Library
- **queries-service**: Test endpoints with Hono's test client

---

## 7. Communication Protocol

### 7.1 What Agents Report to the Orchestrator

After completing a task, every agent must report:

1. **Files changed** — Full list of files created, modified, or deleted
2. **Documentation updates** — Which knowledge base files were updated and why
3. **Cross-domain impacts** — Any changes that affect other domains' interfaces
4. **Test results** — Whether tests pass
5. **Open questions** — Anything the agent is uncertain about

### 7.2 When to Escalate to the User

- Ambiguous ownership (file doesn't clearly belong to any domain)
- Conflicting invariants between domains
- A task that requires breaking a documented invariant
- Any destructive operation (file deletion, data migration, etc.)
- Git operations of any kind

---

## 8. Knowledge Base Maintenance

### 8.1 The Self-Improving System

Every session should leave the knowledge base richer than it found it. This is not
optional — it is the core value proposition of the agent engineering organization.

The post-task documentation protocol (Section 2) is the mechanism. The review bot
(Phase 6) is the enforcement.

### 8.2 Knowledge Base File Purposes

| File | Contains | Updated When |
|------|----------|-------------|
| OWNERSHIP.md | File/directory ownership, glob patterns, shared files | New files created, ownership boundaries change |
| ARCHITECTURE.md | Design decisions, component relationships, invariants, technical debt | Architecture decisions made, invariants discovered, debt identified |
| CONVENTIONS.md | Naming patterns, error handling, anti-patterns, how-to guides | New patterns established, new anti-patterns discovered |
| BUGS.md | Bug fix history with symptoms, root causes, fixes, prevention | Every bug fix (mandatory) |
| INTERFACES.md | Public APIs, consumed interfaces, shared types, evolution rules | Any interface change (mandatory) |

### 8.3 Quality Over Quantity

- One deeply documented bug with actionable prevention is worth more than ten shallow entries
- One real architecture decision with rationale is worth more than a page of "this module does X"
- If you can't explain WHY something is the way it is, ask the user rather than guessing

---

## 9. Domain Agent Context Loading

When the orchestrator spawns a domain agent via the Task tool, it loads:

1. **This handbook** (always)
2. **The domain's full knowledge base** (OWNERSHIP.md, ARCHITECTURE.md, CONVENTIONS.md,
   BUGS.md, INTERFACES.md)
3. **Adjacent domains' INTERFACES.md** (if the task touches cross-domain boundaries)
4. **The specific task description** with scoped requirements

The agent operates within its domain boundary using this context. It does not need to
(and should not) read code outside its ownership boundary unless specifically investigating
a cross-domain interface issue.
