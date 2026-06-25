# ADR-0002: Bytecode VM vs. tree-walker (go/no-go for R6)

- **Status:** Proposed — **awaiting human design review** (go/no-go gate)
- **Date:** 2026-06-25
- **Context slice:** Bytecode VM (issue #11)

## Context

quick-swift currently executes Swift by **tree-walking** msf's typed AST
(`crates/quick-swift-core/src/interp.rs`, ~4.4k LOC). Phases R0–R5 (lexical →
errors/modules) are implemented this way. Issue #11 proposes the **R6 register
bytecode VM** described in the design research docs:

> AST → IR (basic blocks, virtual registers) → liveness + graph-colouring
> register allocation → fixed-width register bytecode → `match`-dispatch VM loop
> → (later) computed-goto; move exception handling to a stack-marker scheme;
> design frames so they can be **saved and resumed**.

Two distinct motivations are bundled into #11:

1. **Throughput** — replace tree-walking on the hot path.
2. **Suspendable frames** — a save/restore execution state primitive that
   `async`/`await` (#12) and generators fundamentally require. A C-stack
   tree-walker cannot suspend mid-evaluation; the register VM gives this
   "don't free the registers, remember the pc" capability cheaply.

The project's own design doc is explicit about sequencing
(`docs/research/2026-06-24-implementing-a-swift-runtime-on-msf-ast.md`, §11/§14):

> "Get it running first, then make it fast." … "run R0–R5 before deciding
> whether the R6 register VM is worth it. The bytecode VM is a later throughput
> optimisation, **not a prerequisite**."

This makes #11 a genuine **go/no-go architectural decision**, not a foregone
implementation task. Building a second execution engine that must reach **full
feature parity** with the tree-walker (all R0–R5 golden fixtures, identical
output) before it is useful is a large, high-risk commitment. It should not be
started until the decision is ratified by a human reviewer, with evidence.

## Decision (proposed)

This ADR records the decision **framework, design, and recommendation**. The
binary go/no-go is reserved for human design review.

### The two motivations decouple — and should be decided separately

| Motivation | Needs the full register VM? | Driver |
| --- | --- | --- |
| Throughput | Yes (IR + reg-alloc + bytecode loop) | Only if benchmarks show the tree-walker is a real bottleneck for target workloads |
| Suspendable frames | **A suspension mechanism**, not necessarily a *register-allocating* VM | Hard requirement for `async`/`await` (#12) and generators |

The recommendation is to **decide throughput on evidence** (benchmarks) and to
**treat suspendable frames as a separate, narrower design problem** that #12 can
also be served by — e.g. a CPS/stackless re-entrant evaluator or a
state-machine lowering — without a full graph-colouring register allocator.

### Go criteria (build the register VM) — all should hold

- [ ] Benchmark suite shows the tree-walker is a **material bottleneck** for a
      realistic target workload (not microbenchmarks alone).
- [ ] The expected speedup (informed by the MiniJS/QuickJS studies, typically
      2–10× for register VMs over naive tree-walkers) justifies maintaining a
      **second execution engine** and keeping it at feature parity.
- [ ] We accept the parity tax: every future language feature must be
      implemented in *both* engines until the tree-walker is retired, OR we
      commit to a hard cutover with a parity gate.

### No-go criteria (stay tree-walker, solve suspension narrowly) — any holds

- [ ] Benchmarks show tree-walking is **not** the bottleneck for target
      workloads (I/O-bound, small programs, test/playground use).
- [ ] The only pressing need is suspension for #12, which a narrower mechanism
      can satisfy without a register allocator.
- [ ] Team bandwidth cannot sustain two engines at parity.

### Proposed VM design (if "go")

Mirrors MiniJS §3–§6 / QuickJS §4–§8, specialised for Swift's **typed** AST:

1. **IR**: lower hot AST subtrees → basic blocks with unlimited virtual
   registers; `node->type` keeps it monomorphic so we skip dynamic-type
   discovery.
2. **Register allocation**: liveness analysis → graph-colouring allocator →
   fixed-width register bytecode; backpatch forward jumps.
3. **VM loop**: `match`-dispatch first (computed-goto-style later behind a
   feature flag).
4. **Exceptions**: move from completion-signal propagation to QuickJS's
   **stack-marker** scheme.
5. **Suspendable frames**: frame = saved registers + pc + scope chain; suspend
   = stop polling and retain the frame; resume = restore and continue. This is
   the primitive #12/generators consume.
6. **Rollout**: VM behind a `--vm` CLI flag and a cargo feature; the existing
   golden-fixture harness runs the **entire R0–R5 corpus through both engines**
   and asserts byte-identical stdout (the parity gate). Ship only after parity
   + measured speedup.
7. **Scope add-ons** that benefit from the VM (per #11): key paths
   `\Root.path`, `consume`/`borrow`, `~Copyable`/`~Escapable`.

## Consequences

- **Good:** the decision is made on data, not vibes; the design is written down
  so a "go" can start immediately; suspension (the #12 blocker) is separated
  from the throughput question so #12 is not hostage to a full VM.
- **Cost / open risk:** the **parity tax** of two engines is the dominant
  long-term cost. The go/no-go must weigh it explicitly.
- **This ADR does not authorise building the VM.** It authorises (a) recording
  the decision framework and (b) standing up the **benchmark baseline** that the
  go/no-go needs. See the companion benchmark suite
  (`crates/quick-swift-cli/benches/`).

## What this slice delivers (issue #11, partial)

- ✅ **Architectural decision *recorded*** (this ADR) — the first acceptance
  criterion's artifact, in `Proposed` status pending review.
- ✅ **Benchmark suite** establishing the tree-walker baseline — the prerequisite
  for the "measurable speedup vs tree-walker" criterion and the evidence the
  go/no-go depends on.
- ⛔ **Blocked on human design review** (go/no-go) before the full pipeline,
  feature-parity VM loop, suspendable frames, and key-paths/ownership work
  begin. These remain open per #11.

## Notes

- Decoupling suspension from throughput directly de-risks #12 (Concurrency),
  which was `Blocked by #11`. **ADR-0003** records the recommended decoupling:
  stackful coroutines as the suspension primitive, independent of this VM
  go/no-go. It is `Proposed` pending a de-risking spike, after which #12 no
  longer depends on #11. See
  `docs/research/2026-06-25-suspendable-frames-implementation-options.md` for the
  full menu of alternatives considered.
