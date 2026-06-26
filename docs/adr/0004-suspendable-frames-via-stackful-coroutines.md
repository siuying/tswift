# ADR-0004: Suspendable frames via stackful coroutines

- **Status:** Accepted
- **Date:** 2026-06-25
- **Context slice:** Concurrency (issue #12); unblocks it from issue #11
- **Supersedes for suspension:** the implicit assumption in #12 that suspension
  requires the #11 bytecode VM

## Context

Swift structured concurrency (#12) — `async`/`await`, `async let`, task groups,
actor reentrancy, generators — requires a **suspension primitive**: save the full
execution state at an `await`, return to a scheduler, and resume later exactly
where execution paused.

Our evaluator (`crates/qswift-core/src/interp.rs`) is a **tree-walker** that
recurses on the **Rust call stack**. At an `await`, the in-flight evaluation chain
lives in native stack frames that cannot be frozen — so #12 was modelled as
**blocked by #11** (the register bytecode VM, whose frames are explicitly
save/restore-able).

ADR-0002 recommended **decoupling suspension from throughput**: the VM is a
*performance* decision (still pending human go/no-go), whereas suspension is a
*capability* that a narrower mechanism can provide without a second execution
engine. The companion research note
(`docs/research/2026-06-25-suspendable-frames-implementation-options.md`)
enumerates five alternatives and compares them.

## Decision

**Implement the suspension primitive with stackful coroutines** (Option 1): run
each Swift `Task` on its **own native stack** via a stack-switching library
(target: [`corosensei`](https://docs.rs/corosensei)). Suspending parks the task's
native stack and switches to the scheduler; resuming swaps back and recursion
continues unchanged.

This is adopted **independently of the #11 bytecode-VM go/no-go**, which removes
the #11 → #12 blocking dependency.

### Why this option

- **Zero changes to the proven tree-walker.** The recursive evaluator runs as-is
  on the coroutine stack; we do not rewrite `eval`.
- **No borrow-checker fight.** Because Rust frames pause in place, no `&mut self`
  / `&mut dyn Write` / env borrow is held *across* an await in Rust's view. The
  `async fn` and explicit-state-machine options (Options 4/3) struggle here.
- **Lowest effort** to a working suspension primitive; least new surface to
  maintain.
- **Decouples capability from performance.** #12 proceeds now; the VM remains a
  separate throughput decision.

### Scope boundary

This ADR covers only the **suspension primitive** (save/resume a running
evaluation). **Scheduling** — the child-task tree, cooperative cancellation,
`@MainActor`/actor serialization, `Sendable` checking, ordering — is a **separate
custom cooperative executor** built *on top of* this primitive, tracked under #12.
Suspension ≠ scheduling.

## Consequences

- **Good:** #12 is unblocked from #11; minimal, low-risk addition; the
  tree-walker stays the single source of truth for language semantics (no parity
  tax of two engines).
- **Cost / risk:** one native stack per *live* task (a few KB; mitigated by
  growable/segmented stacks); the coroutine crate carries `unsafe` stack
  switching (encapsulated, not in our evaluator); native-stack switching has
  platform/unwinding considerations — mitigated by choosing a crate
  (`corosensei`) that handles `catch_unwind`/portability.
- **Migration path:** if we later need to drop the native-stack dependency or
  want maximum fidelity to Swift's heap-allocated async frames, move the async
  path to **Option 2** (selective CPS / state-machine lowering of async bodies),
  per the research note. The register VM (#11) stays an independent decision.

## Notes

- The `unsafe` confinement principle from ADR-0001 is preserved: stack-switching
  `unsafe` lives behind the coroutine crate's safe API, not in
  `qswift-core`.
- Acceptance for the *primitive* (separate from #12's full executor): a trivial
  Swift `await` round-trips through the unchanged tree-walker by suspending and
  resuming across a scheduler boundary — a good first de-risking spike.
