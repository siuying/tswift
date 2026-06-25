# Suspendable frames: implementation options for a tree-walker

**Date:** 2026-06-25
**Context:** issue #11 (Bytecode VM), issue #12 (Concurrency), ADR-0002, ADR-0003
**Cross-ref:** `docs/research/2026-06-24-implementing-a-swift-runtime-on-msf-ast.md` §13,
`docs/research/2026-06-24-minijs-building-a-vm-for-javascript.md` (save-frame model)

## The problem in one sentence

Our evaluator (`crates/quick-swift-core/src/interp.rs`) runs Swift by **recursive
descent on the Rust call stack**: `eval(node, env)` calls itself for sub-expressions.
At a Swift `await`, the entire chain of in-flight evaluations lives in **native Rust
stack frames** we cannot freeze, copy, or walk away from. To implement `async`/`await`,
`async let`, task groups, generators, and actor reentrancy (#12) we need a **suspension
primitive**: *save the whole execution state, return to a scheduler, and resume later
exactly where we left off.*

The MiniJS/QuickJS recipe gets this from a **register bytecode VM** — "don't free the
registers, remember the pc" (#11). But that is the *cheapest-per-suspend* answer, not the
*only* answer, and it costs a second execution engine. This document enumerates the
realistic alternatives for a **tree-walker** and records why we pick stackful coroutines.

Each option is a different answer to the question: **where do we put the saved state so we
can leave and come back?**

---

## Option 1 — Stackful coroutines (chosen)

Give each Swift `Task` its **own native stack**. The tree-walker runs on that stack
unchanged; suspending **parks the whole native stack** and switches to the scheduler's
stack. Resuming swaps back and recursion continues as if nothing happened.

- **Mechanism:** a stack-switching crate such as [`corosensei`](https://docs.rs/corosensei)
  (modern, sound, actively maintained), or `generator`/`may`; under the hood this is
  `swapcontext`-style native stack switching.
- **"Remember the pc" analogue:** *don't free the native stack — remember to swap back to it.*
- **Key advantage for us:** the tree-walker code is **completely unchanged**, and it
  **sidesteps the borrow checker**. No `&mut self`, `&mut dyn Write`, or env-chain borrow
  is ever *held across* an await in Rust's view, because the Rust frames simply pause in
  place. Every option that rewrites `eval` (3, 4) fights this borrow problem hard.
- **Cost:** one stack allocation per *live* task (a few KB; growable/segmented stacks keep
  this small), and `unsafe` stack switching — but that `unsafe` is encapsulated inside the
  coroutine crate, not in our evaluator.
- **Caveats:** native-stack switching has platform/portability and unwinding/`catch_unwind`
  considerations; pick a crate that handles these (corosensei does).
- **Effort:** **Low.** This is how many embeddable interpreters add coroutines.

## Option 2 — Selective CPS / state-machine lowering of `async` functions only

Do what Swift's real compiler does: lower **only `async` function bodies** into resumable
state machines. Split each async body at its `await` points into segments and heap-allocate
an **async frame** (locals + a resume index) that links up the task chain. Synchronous code
stays on the ordinary recursive tree-walker.

- **"Remember the pc" analogue:** the resume index *is* the pc; the heap frame *is* the
  saved registers.
- **Most Swift-faithful:** matches Swift's heap-allocated async frames, cooperative
  reentrancy, and child-task trees; no native-stack tricks; no second engine.
- **Cost:** implement an AST → state-machine **split transform** for async bodies, and
  thread frames through async calls. Synchronous performance is untouched.
- **Effort:** **Medium.** Best fidelity-per-effort if/when we want to drop the native-stack
  dependency. A natural **follow-on** to Option 1.

## Option 3 — Stackless re-entrant evaluator (explicit work stack / trampoline)

Rewrite the evaluator so Swift-level calls **do not use the native stack**. Maintain an
explicit `Vec<Frame>` where each frame is an AST node plus a continuation/PC into it; an
outer loop drains the stack. Suspension = stop draining and keep the `Vec`.

- This is **halfway to the bytecode VM**: suspendable frames without bytecode, register
  allocation, or a second engine — but you rewrite `eval` as an explicit state machine over
  AST nodes (a "tree-walking VM" with a manual work stack).
- **Cost / effort:** **High** — a near-total rewrite of `interp.rs`, and *slower* than
  direct recursion on the non-async path. Mainly worth it only if we also intend to build
  the VM (#11) later, as a stepping stone.

## Option 4 — Host async (`eval` becomes an `async fn`)

Make the evaluator itself `async` so Swift `await` maps to Rust `.await` and **rustc
generates the state machine for us**.

- **"Remember the pc" analogue:** rustc remembers it — the `Future` *is* the saved frame.
- **Worst fit here in practice:** recursive `async fn` requires boxing (`BoxFuture` /
  `async-recursion`), so **every `eval` call heap-allocates a future** (heavy), and holding
  `&mut self` / env borrows across `.await` across recursion is a constant borrow-checker
  fight. You also still need a custom cooperative executor on top to get Swift's
  child-task/cancellation semantics, so it doesn't even save the scheduler work.
- **Effort:** low *code* volume, high *friction*; rejected.

## Option 5 — OS threads + blocking (spike only)

Run each task on a real OS thread, blocking on a channel at `await`.

- Quick to prototype, but **not** cooperative single-threaded scheduling: it breaks actor
  reentrancy/ordering fidelity and muddies `Sendable`/data-race semantics. Useful as a
  throwaway spike to validate the executor API shape; **not** a faithful executor.

---

## Comparison

| Approach | Effort | Per-suspend cost | Borrow-checker friction | Swift fidelity | Touches hot sync path? |
|---|---|---|---|---|---|
| **1. Stackful coroutines** | Low | One parked stack / task | **None** (code unchanged) | High | No |
| **2. Selective CPS lowering** | Medium | One heap frame / await | Low | **Highest** | No |
| **3. Stackless work-stack rewrite** | High | Cheap (it is the design) | N/A (rewrite) | High | Yes (slower) |
| **4. Host `async fn` eval** | Low code / high friction | Boxed future / call | **Severe** | Medium | Yes (heavy) |
| **5. OS threads** | Trivial | A thread / task | None | Low | No |
| *Register VM (#11/ADR-0002)* | *Very high* | *Cheapest (regs + pc)* | *N/A* | *High* | *No (new engine)* |

## Decision

**Go with Option 1 (stackful coroutines).** See **ADR-0003** for the full rationale and
consequences. In short: it delivers the suspension primitive #12 needs with the **least
code and zero changes to the proven tree-walker**, and it **decouples suspension from the
bytecode-VM throughput decision** (ADR-0002) — so #12 is no longer hostage to #11's
go/no-go. Scheduling concerns (child-task tree, cancellation, actor serialization) are a
**separate** custom cooperative executor built *on top of* this primitive.

**Migration path:** if we later need to drop the native-stack dependency or want maximum
fidelity to Swift's heap async frames, move the async path to **Option 2** (selective CPS
lowering). The register VM (#11) remains an independent *throughput* decision.
