# ADR-0005: Cooperative single-threaded concurrency executor

- **Status:** Accepted
- **Date:** 2026-06-25
- **Context slice:** Concurrency (issue #12)
- **Builds on:** ADR-0004 (suspension primitive via stackful coroutines)

## Context

Issue #12 asks for Swift structured concurrency — `async`/`await`, `async let`,
`Task`/`Task.detached`, `withTaskGroup`, `actor` isolation, `@MainActor`,
`Sendable`, `AsyncSequence`/`for await` — "running on a Swift-faithful cooperative
executor over the VM's suspendable frames."

ADR-0004 already decided the **suspension primitive**: each suspendable unit runs
on its own native stack via [`corosensei`](https://docs.rs/corosensei), so the
recursive tree-walker (`crates/tswift-core/src/interp.rs`) can pause at an
`await` and hand control back to a scheduler without unwinding. This ADR records
the **scheduler/executor** built on top of that primitive — the design-decision
gate that issue #12 calls out as its first acceptance criterion.

## Decision

Implement a **custom, single-threaded, cooperative executor** (not tokio, not OS
threads) that matches Swift's structured-concurrency model:

1. **Tasks are coroutines.** Every `Task`, `Task.detached`, `async let` child,
   task-group child, and the `@main async` entry point runs as a `corosensei`
   coroutine that captures a stable raw pointer to the `Interpreter`. The
   tree-walker evaluates the task body unchanged on that coroutine's stack.
2. **One scheduler loop owns resumption.** A single driver loop in the executor
   is the *only* place that resumes coroutines. Coroutines never resume each
   other; they **suspend back to the loop** with a reason
   (`Await(id)` / `Yield`), and the loop decides what runs next. This keeps all
   `unsafe` stack-switching at one well-defined boundary and avoids re-entrant
   borrows of the interpreter.
3. **`await` only suspends on a pending task.** `await f()` where `f` is an
   `async` function is an ordinary call evaluated inline on the current task's
   stack (it may itself suspend). Suspension happens only when awaiting a
   **task handle** (`async let` binding, `Task.value`, group results) that has
   not completed: the awaiting task yields `Await(id)`, the loop drives `id` to
   completion, then resumes the waiter with the result.
4. **Single thread ⇒ actors serialize for free.** Because exactly one coroutine
   runs at any instant and there is no preemption, `actor` isolation and
   `@MainActor` are satisfied structurally: mutable actor state is never touched
   concurrently. Actors are modelled as reference types whose async members run
   as tasks; `@MainActor`/global-actor annotations are accepted and run on the
   same cooperative thread.
5. **Cooperative cancellation is a flag.** A task carries an `isCancelled` flag
   propagated to its structured children. `cancelAll()` / task-group teardown set
   it; `Task.isCancelled` / `Task.checkCancellation()` observe it. Cancellation
   is cooperative — it never force-unwinds a running coroutine.

### Fidelity boundary (explicitly accepted)

Per the issue ("aim for semantic fidelity, not scheduler-identical ordering"):

- **Results and structure are faithful:** child tasks run and are awaited; task
  trees nest; cancellation propagates; actor state stays consistent.
- **Interleaving order may differ** from Apple's executor. We run a ready task to
  a suspension point before switching, and we resume waiters in FIFO order. Any
  program whose output depends on a *specific* preemption order (e.g. racing
  `Task.yield()` loops) may print in a different but internally-consistent order.
  Fixtures avoid asserting on such races.

## Consequences

- **Good:** semantically faithful structured concurrency with one execution
  engine (the tree-walker), one `unsafe` boundary (the coroutine crate), and no
  data races by construction (single thread). Independent of the #11 bytecode-VM
  go/no-go.
- **Cost / risk:** one native stack per *live* task; ordering is not
  bit-identical to Apple's runtime (documented above); true parallelism is out of
  scope (cooperative only).
- **Migration path:** if real parallelism or Apple-identical ordering is ever
  required, the executor loop is the single place to evolve (e.g. multiple worker
  stacks); the `await`/spawn surface in the interpreter would not change.

## Notes

- `unsafe` confinement (ADR-0001) is preserved: stack switching lives in
  `corosensei`; the executor adds only a stable-pointer/thread-local-yielder
  boundary, encapsulated in `crates/tswift-core/src/concurrency.rs`.
- Acceptance fixtures live under `crates/tswift-cli/tests/fixtures/` and
  cover `async`/`await`, `async let`, `Task`/`withTaskGroup`, and `actor`.
</content>
</invoke>
