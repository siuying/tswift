# ADR-0007: Interpolation fragments use an interpreter-owned, reclaimable cache

- **Status:** Accepted
- **Date:** 2026-06-28
- **Context slice:** Embedding the runtime as a long-running native host
  (`TSwiftCore`/`TSwiftUI`); prerequisite for the Swift-library work.
- **Amends:** ADR-0001 (unsafe confined to the FFI layer), ADR-0003 (interpolation
  analyses leaked forever).

## Context

ADR-0003 made the interpreter operate on `Node<'static>` and leak every
string-interpolation fragment's `Analysis` (`eval_interpolation`,
`crates/tswift-core/src/interp.rs`). That is bounded and harmless for the
run-once CLI, but ADR-0003 explicitly said to **revisit if the runtime is ever
embedded as a library or long-running/REPL host**. The `TSwiftUI` Playground is
exactly that host: it re-renders `body` on every interaction (leaking one
`Analysis` per `"\(expr)"` *per render*) and recompiles on every keystroke
(creating a fresh interpreter whose leaks are never reclaimed). Left as-is,
memory grows without bound both within and across sessions.

We pivoted to fix this leak **before** building `TSwiftCore`/`TSwiftUI`, so the
native host embeds a runtime that does not grow memory while it runs.

## Decision

Replace the per-evaluation `Box::leak` with an **interpreter-owned, append-only,
source-keyed fragment cache** (`interp/fragment_cache.rs`):

- `entries: Vec<Box<Analysis>>` + `index: HashMap<String, usize>`. The `Box`
  indirection gives each `Analysis` a stable heap address, so a `&'static`
  handed out earlier stays valid as later fragments are pushed (the `Vec` moves
  box pointers, never the `Analysis`).
- `get_or_analyze(src) -> Result<&'static Analysis>`: cache hit returns the
  stored `'static` reference; miss analyzes once, boxes, pushes, and transmutes
  `&*entry` to `'static`.
- The cache is a field of `Interpreter`; `Drop` frees every fragment with the
  session. No leak within a session (a repeated fragment is analyzed once) **or**
  across sessions (a dropped interpreter reclaims its cache).

### Two amendments this forces

1. **ADR-0001 (unsafe confinement).** The `'static` transmute is a small `unsafe`
   seam inside `tswift-core`, which ADR-0001 declared 100%-safe above the FFI
   layer. We relax that rule to: `unsafe` is confined to the FFI layer **and** this
   one documented self-referential cache module. Enforcing 100% safety was always
   going to bend at the embedding boundary; bending it deliberately in one audited
   place is the honest cost.
2. **ADR-0003 (leak forever).** Interpolation analyses are no longer leaked for
   the process lifetime; they are owned and reclaimed. ADR-0003's `'static`
   *node* model is unchanged — fragments are still `Node<'static>`; only their
   *storage* moved from "leaked" to "owned by the interpreter."

## Soundness

The transmute to `'static` is sound because:

- `Node<'a>` is `#[derive(Clone, Copy)]` with **no `Drop`** — stored
  `Node<'static>` cursors never dereference their analysis on drop, so
  interpreter field drop-order is irrelevant.
- The cache is **append-only and never evicts** — this is a *requirement*, not a
  convenience: removing an entry while a `Node<'static>` still points into it
  would be UB. Distinct fragments are bounded by program text, so the cache
  plateaus.
- Cache boxes outlive every render that uses them; they are freed only when the
  interpreter (their sole owner) drops.

## Consequences

- **Good:** a long-running, repeatedly-recompiling host runs in bounded memory;
  the interpolation path is faster (repeated fragments analyzed once); the
  embedding prerequisite is retired before any Swift binding is written.
- **Cost:** one audited `unsafe` module in the former all-safe core; the
  no-eviction invariant must be preserved by future edits (documented in-module).
