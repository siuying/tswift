---
name: autoloop
description: Autonomous developer iteration loop driven by a thin orchestrator that dispatches coder/reviewer/diagnoser subagents. Pick a mode — fix (make tests green), feature (implement one or more features), optimize (push a metric), debug (root-cause a failure), ab-test (race candidate implementations), or prototype (explore design variants) — define a verification signal, confirm the plan with the user, then loop implement → verify → review → commit with all state kept in loop-log.md. Use when the user states a concrete engineering goal or says "keep iterating", "loop on this", "autoloop", or "iterate until fixed".
---

# Autoloop

A developer iteration loop for concrete engineering work: fixing bugs, shipping
features, debugging failures, optimizing hot paths, racing alternative
implementations, and prototyping designs.

A thin **orchestrator** drives the loop. Heavy work (coding, reviewing,
diagnosing) runs in **fresh-context subagents** that return short, structured
results. This keeps orchestrator context growth linear (~200 tokens/iteration)
so the loop can run indefinitely without degrading.

## Roles

| Role               | Context             | Reads code?                       | Returns                        |
| ------------------ | ------------------- | --------------------------------- | ------------------------------ |
| Orchestrator       | long-lived, small   | **never**                         | —                              |
| Coder subagent     | fresh per dispatch  | yes (in-scope files + `notes.md`) | ≤10-line status report         |
| Reviewer subagent  | fresh per dispatch  | yes (staged diff)                 | verdict + issue list           |
| Diagnoser subagent | fresh, on crash/bug | yes (`run.log` + code)            | one-line cause + suggested fix |

**Orchestrator I/O is limited to:** `loop-log.md`, `notes.md`, `git` plumbing
commands, running the verification signal, and `grep`/`tail` on `run.log`.
If it needs to understand code or a failure, it dispatches a subagent — it never
reads source files or full logs itself.

## Model assignment

Each role runs on a model tier matched to its difficulty. Cheap, well-scoped
work goes to fast models; judgment-heavy work goes to frontier models.

| Role                     | Difficulty                 | Model tier                                      | Examples             |
| ------------------------ | -------------------------- | ----------------------------------------------- | -------------------- |
| Orchestrator / Planner   | hard (judgment, planning)  | frontier                                        | Fable, Opus, GPT-5.5 |
| Coder subagent           | simple, well-scoped        | fast/cheap                                      | Sonnet, GPT-5.4-mini |
| Survey/research subagent | simple, bounded            | fast/cheap                                      | Sonnet, GPT-5.4-mini |
| Diagnoser subagent       | medium (escalate if stuck) | fast/cheap                                      | Sonnet, GPT-5.4-mini |
| Reviewer subagent        | hard (adversarial)         | frontier, **different provider than the coder** | see rule below       |

**Cross-provider review rule** — the reviewer must be a smart model from a
_different provider_ than the coder, to avoid shared blind spots:

- Coder on Claude (Sonnet/Opus) → review with GPT-5.5
- Coder on OpenAI/Codex (GPT-5.4-mini/GPT-5.5) → review with Opus

**Always confirm model choices with the user before starting the loop** — the
model assignment is part of the Phase 1 plan and must be approved alongside the
mode and signal. Substitute equivalents if a listed model is unavailable.

**Communication protocol — two channels, never mixed:**

1. **Return values are digests** — every subagent reply to the orchestrator is
   short and structured (≤15 lines), containing only what the orchestrator needs
   to make its next dispatch decision. Never diffs, logs, code, or transcripts.
2. **Rich data flows through files** — anything detailed a _later subagent_ needs
   goes to disk: `notes.md` (coder → coder), `loop-log.md` (history), `run.log`
   (signal → diagnoser). Subagents read these directly; the orchestrator just
   routes file names in dispatch prompts.

---

## Modes

Pick the mode first — it fixes the signal shape, the loop body, and the stop
condition.

| Mode          | Goal                                         | Signal                          | Stops when                     |
| ------------- | -------------------------------------------- | ------------------------------- | ------------------------------ |
| **fix**       | make failing tests pass                      | test suite → pass/fail count    | all green                      |
| **feature**   | implement one or more features               | acceptance tests → pass/fail    | all features done + all green  |
| **optimize**  | push a numeric metric (perf, coverage, size) | benchmark/coverage/size command | metric hits target or plateaus |
| **debug**     | root-cause an unexplained failure            | minimal repro command           | cause found + regression test  |
| **ab-test**   | pick the best of N candidate implementations | shared benchmark, same harness  | all candidates measured        |
| **prototype** | explore design variants cheaply              | "it runs" + user eyeball        | user picks a direction         |

Mode-specific loop shape:

- **fix** — classic TDD: failing test first, minimal code to green, refactor.
- **feature** — break the feature list into vertical slices, one per iteration.
  Each slice is TDD'd: write the acceptance/unit tests for the slice first, then
  minimal code to green. A feature counts as done only when its tests pass and
  the full suite stays green.
- **optimize** — one hypothesis per iteration ("cache X", "batch Y"); measure
  before/after on the same benchmark; discard on no measurable gain.
- **debug** — each iteration is a **hypothesis test**: state the hypothesis in the
  log row, add instrumentation or bisect (`git bisect`, binary-search the input),
  run the repro, record confirmed/refuted in `notes.md`. When the cause is found,
  switch to **fix** mode: write the regression test, then the fix.
- **ab-test** — one branch per candidate (`ab/<name>`). Each iteration implements
  or refines one candidate, runs the _identical_ benchmark harness, and logs the
  score. Finish with a comparison table; merge only the winner, delete the rest.
- **prototype** — review gate relaxed (skip the reviewer subagent); commits go to
  a `proto/<name>` branch; throwaway code is fine but each variant must run.

---

## Phase 1 — Scope & Plan

1. **Survey (cheap, bounded):** dispatch one coder subagent to read `AGENTS.md` /
   `README.md` and locate in-scope files (may use `codebase-locator` /
   `codebase-analyzer`). Return contract: `FILES` (with 5-word roles), `FACTS`
   (build/test commands, conventions), `RISKS` (flaky tests, hidden coupling) —
   ≤15 lines, no code listings. This is scoping, not research: time-boxed, and
   only over files plausibly touched by the task.
2. **Pick the mode** (table above) and the **verification signal** — exact
   command + how to parse the metric from its output.
3. **Draft the experiment list** — ordered, simple → complex, each atomic and
   independently verifiable. For **feature**: the ordered slice list. For
   **ab-test**: the candidate list. For **debug**: the initial hypothesis list.
4. Present plan to user and **wait for confirmation**:

```
Mode:           fix | feature | optimize | debug | ab-test | prototype
Problem:        <restate>
Files in scope: <list>
Signal:         <command> → <metric>
Experiments:    1. <idea>  2. <idea>  …
State files:    loop-log.md, notes.md (untracked)
Models:         orchestrator=<model>  coder=<model>  reviewer=<model, different provider>  diagnoser=<model>
```

IMPORTANT: Do not begin the loop until the user confirms both the plan **and** the model
assignment.

---

## Phase 2 — The Loop

Once confirmed, run until the mode's stop condition or the user interrupts.
Gate order is **implement → verify → review → commit** — verification is cheap,
review is expensive, so fail fast on the metric before paying for a review.

### Gate 1 — Implement (coder subagent)

Dispatch a coder subagent with a self-contained prompt:

```
TASK: <one experiment / hypothesis / candidate from the list>
MODE: <mode> — PROBLEM: <original problem statement>
SCOPE: <in-scope files>
CONSTRAINTS: focused atomic change; TDD for behavior changes (failing test first);
  stage with `git add`, do NOT commit.
CONTEXT: read notes.md first. Recent failed ideas: <last N discard reasons>.
BEFORE FINISHING: append any durable insight (flaky tests, hidden coupling,
  gotchas, refuted hypotheses) to notes.md.
RETURN CONTRACT — reply with ONLY:
  STATUS: done | blocked
  FILES: <changed files>
  SUMMARY: <one line>
Do not include diffs, logs, or exploration transcripts.
```

Exception: for trivially small experiments (≈1-line change), the orchestrator may
implement inline to skip the round-trip.

### Gate 2 — Verify (orchestrator, cheap)

- Run the signal: `<command> > run.log 2>&1`
- Extract the metric with `grep`/`tail -n 20` on `run.log`. **Never read the full
  log** — on crash, dispatch a diagnoser subagent instead.
- Compare to the current best (fix/feature/optimize) or record the score (ab-test) or the
  hypothesis verdict (debug). If same/worse → discard now (skip review):
  `git checkout -- . && git clean -fd -- <scope>`; log the reason; next iteration.
- **Optimize/ab-test:** run the benchmark ≥3 times if it's noisy; compare medians.

### Gate 3 — Review (reviewer subagent, only on improvement)

Skipped in **prototype** mode.

- Capture `BASE=$(git rev-parse HEAD)` and the staged diff.
- Dispatch a reviewer subagent using the template at
  [`../requesting-code-review/code-reviewer.md`](../requesting-code-review/code-reviewer.md),
  filling `[DESCRIPTION]`, `[PLAN_OR_REQUIREMENTS]`, `[BASE_SHA]`, `[HEAD_SHA]`.
- Return contract: verdict (pass/fail) + list of Critical/Important/Minor issues,
  ≤10 lines.
- Critical/Important → re-dispatch the coder with the issues to fix (once);
  unfixable → discard. Minor → note in log, continue.

### Gate 4 — Commit or Discard (orchestrator)

| Outcome                           | Action                                                     | Status  |
| --------------------------------- | ---------------------------------------------------------- | ------- |
| Metric improved and review passed | `git commit -m "<type>(<scope>): <experiment>"`            | keep    |
| Metric same/worse                 | discard working tree changes                               | discard |
| Crash / timeout                   | diagnoser subagent → one-line cause; fix trivially or skip | crash   |
| Review Critical unfixable         | discard                                                    | discard |
| ab-test candidate measured        | commit on its `ab/<name>` branch                           | scored  |
| debug hypothesis refuted          | discard instrumentation, note verdict                      | refuted |

After every iteration, append one row to `loop-log.md` — **including a one-line
"why it failed" for discards** so future coder dispatches don't retry blind.

---

## State files (disk is the source of truth)

All live in the project root, untracked (add to `.gitignore` if needed).
**After a row is logged, the iteration's transcript is never needed again** — this
makes the orchestrator compactable and resumable: on restart, rebuild state from
`loop-log.md` + `git log` and continue.

### loop-log.md

```markdown
# Autoloop Log

**Mode**: optimize
**Problem**: <original problem statement>
**Signal**: `<command>` → <metric name>
**Baseline**: <initial metric value>

| #   | commit  | metric | Δ     | review | status  | description / discard reason |
| --- | ------- | ------ | ----- | ------ | ------- | ---------------------------- |
| 1   | a1b2c3d | 142ms  | +0.0% | pass   | keep    | baseline                     |
| 2   | —       | 149ms  | −4.9% | —      | discard | memoize Y — cache misses     |
```

For **ab-test**, end with a comparison table:

```markdown
| candidate   | branch  | metric | verdict |
| ----------- | ------- | ------ | ------- |
| lru-cache   | ab/lru  | 98ms   | winner  |
| ring-buffer | ab/ring | 121ms  | drop    |
```

### notes.md

Coder-to-coder scratchpad of durable insights ("test X is flaky", "module Y has
hidden coupling with Z", "hypothesis: lock contention — refuted"). Each coder
reads it on start and appends on finish. Orchestrator includes nothing from it
in its own context.

---

## Rules

**Simplicity criterion** — all else equal, simpler is better:

- Marginal gain + added complexity → discard
- Zero gain + simpler code → keep (simplification win)
- Clear gain → keep regardless of complexity

**Never stop to ask** whether to continue. If out of ideas: dispatch a fresh
coder subagent to re-read the in-scope files plus `loop-log.md` + `notes.md`
and propose new experiments — combine near-misses, or go more radical.

**Fair comparison** (optimize / ab-test) — same machine, same harness, same
inputs, warmed-up runs. Never compare numbers taken under different conditions.

**Timeout** — if a verification run exceeds 5× its normal duration, kill it and
treat as crash.

**Context discipline** — if the orchestrator context grows anyway, compact: keep
only the plan, current best, and the last few log rows; everything else is on disk.

---

## Stop Conditions

Stop at the mode's stop condition (see Modes table) or when the user interrupts.
prepare a summary with the goal, the signal, a summary of the iterations and
detailed result as a table in following format:

```
Mode: M  |  Iterations: N  |  Best metric: X  |  Kept: K  |  Discarded: D  |  Crashed: C
```

For **ab-test**, include the comparison table and the winning branch.
For **debug**, include the confirmed root cause and the regression-test commit.

Create a simple html document in temporary folder and open it on broswer.
