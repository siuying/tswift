---
name: autoloop
description: Autonomous iterative problem-solving loop. Given any problem (fix a bug, implement a feature, improve test coverage, boost performance, etc.), the agent surveys the codebase to devise a strategy, proposes a verification signal (test suite, benchmark, build, linter), asks the user to confirm the plan, then loops — each iteration follows implement → review → verify → commit, keeping improvements and discarding regressions, with every attempt bookept in loop-log.md. Use when the user states any open-ended improvement goal or says "keep iterating", "loop on this", "autoloop", or "iterate until fixed".
---

# Autoloop

An autonomous iterate-and-bookkeep loop. Each iteration goes through four gates before a commit lands: **implement → review → verify → commit**.

## Phase 1 — Survey & Plan

1. Read `AGENTS.md`, `README.md`, or equivalent project root docs.
2. Locate files relevant to the stated problem (use `codebase-locator` skill if helpful).
3. Determine the **verification signal** — command + metric that proves progress:
   - Bug / feature → test suite; signal = pass/fail or test count
   - Coverage → `cargo tarpaulin` / `pytest --cov`; signal = coverage %
   - Performance → benchmark command; signal = numeric score
   - Build / lint → `cargo build` / `ruff`; signal = exit code
4. Determine the **iteration strategy**: ordered list of experiment ideas, simple → complex.
5. Present plan to user and **wait for confirmation**:

```
Problem:        <restate>
Files in scope: <list>
Signal:         <command> → <metric>
Strategy:       1. <idea>  2. <idea>  …
Log file:       loop-log.md (untracked)
```

---

## Phase 2 — The Loop

Once confirmed, run forever until user interrupts or problem is solved.

### Each iteration runs four gates in order:

```
┌─────────────────────────────────────────────────────┐
│  IMPLEMENT → REVIEW → VERIFY → COMMIT               │
└─────────────────────────────────────────────────────┘
```

#### Gate 1 — Implement

- Pick the next experiment idea from the strategy list (generate new ones when exhausted).
- Make a focused, atomic change to the in-scope files.
- Follow TDD when adding/fixing behavior: write a failing test first, then the minimal code to pass it.
- `git add` the changes (do **not** commit yet).

#### Gate 2 — Review

- Capture current SHAs: `BASE=$(git rev-parse HEAD)`, `HEAD_STAGED=$(git stash create)`.
- Dispatch a `general-purpose` code reviewer subagent using the template at  
  [`../../requesting-code-review/code-reviewer.md`](../../requesting-code-review/code-reviewer.md), filling:
  - `[DESCRIPTION]` — what this experiment tries
  - `[PLAN_OR_REQUIREMENTS]` — the problem statement + strategy
  - `[BASE_SHA]` — `$BASE`
  - `[HEAD_SHA]` — staged diff or stash SHA
- Act on feedback:
  - **Critical** → fix before proceeding (or discard the idea entirely)
  - **Important** → fix before proceeding
  - **Minor** → note in log, continue

#### Gate 3 — Verify

- Run the verification signal: `<command> > run.log 2>&1`
- Parse the metric from `run.log`.
- Compare to the **current best**.

#### Gate 4 — Commit or Discard

| Outcome | Action |
|---------|--------|
| Signal improved **and** review passed | `git commit -m "<type>(<scope>): <experiment>"` → status = **keep** |
| Signal same/worse | `git checkout -- .` (unstage/discard) → status = **discard** |
| Crash / timeout | Diagnose briefly; fix trivially or skip → status = **crash** |
| Review Critical unfixable | Discard idea → status = **discard** |

After each iteration, append a row to `loop-log.md`.

---

## Bookkeeping — loop-log.md

Create in the project root; do **not** commit it (add to `.gitignore` if needed).

```markdown
# Autoloop Log

**Problem**: <original problem statement>
**Signal**: `<command>` → <metric name>
**Started**: <ISO timestamp>
**Baseline**: <initial metric value>

## Iterations

| # | commit  | metric | Δ      | review | status  | description                     |
|---|---------|--------|--------|--------|---------|---------------------------------|
| 1 | a1b2c3d | 72.3%  | +0.0%  | pass   | keep    | baseline                        |
| 2 | b2c3d4e | 74.1%  | +1.8%  | pass   | keep    | add tests for edge case X       |
| 3 | —       | 73.0%  | −1.1%  | pass   | discard | refactor Y (coverage dropped)   |
| 4 | —       | —      | —      | fail   | discard | idea Z (critical review issue)  |
| 5 | —       | —      | —      | —      | crash   | OOM in benchmark                |
```

---

## Rules

**Simplicity criterion** — All else equal, simpler is better:
- Marginal gain + added complexity → discard
- Zero gain + simpler code → keep (simplification win)
- Clear gain → keep regardless of complexity

**Never stop to ask** if you should continue. If out of ideas, re-read the codebase, combine near-misses, or try more radical changes.

**Timeout** — If a verification run exceeds 5× its normal duration, kill it and treat as crash.

---

## Stop Conditions

Stop when:
- Problem fully solved (all tests green, metric hits goal, etc.)
- User manually interrupts

On stop, print a final summary:
```
Iterations: N  |  Best metric: X  |  Kept: K  |  Discarded: D  |  Crashed: C
```
