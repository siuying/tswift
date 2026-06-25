# ADR-0003: Interpreter AST lifetime is `'static` (leaked), to support string interpolation

- **Status:** Accepted (implemented)
- **Date:** 2026-06-25
- **Context slice:** Evaluator core (`crates/quick-swift-core/src/interp.rs`), msf safe wrapper

## Context

The `msf` safe wrapper hands out `Node<'a>` borrowed from an `Analysis` value:
the arena-allocated AST and `TypeInfo` graph live exactly as long as the owning
`Analysis`. Early milestones (literals → structs, issues #2–#5) built the
interpreter generically over that borrow lifetime `'a`.

String interpolation broke that model. A literal like `"sum is \(a + b)"`
contains embedded *expression fragments* (`a + b`) that must be evaluated in the
**current environment**, reusing the interpreter's live type tables, function
registry, struct/enum/class definitions, and so on. The natural implementation
re-analyzes each fragment as its own tiny program and walks the resulting nodes
through the same interpreter. But those fragment ASTs are owned by *new*
`Analysis` values created mid-evaluation — their `Node<'_>` cannot be mixed with
the root program's `Node<'a>` under one concrete lifetime without the borrow
checker rejecting it, and they must outlive the single statement that produced
them.

This surfaced at issue #6 and forced a refactor of code already written against
`'a`. Had the lifetime strategy been decided up front, milestones #2–#5 would
have been written correctly the first time.

## Decision

The interpreter operates on `Node<'static>`. Both the **root** `Analysis` and
each **interpolation-fragment** `Analysis` are intentionally leaked
(`Box::leak`), promoting their borrows to `'static`:

- The CLI leaks the root analysis once per program (`Box::leak(Box::new(analysis))`).
- Interpolation re-analysis leaks each fragment analysis so its nodes can be
  evaluated by the same `'static` interpreter and reuse all runtime tables.

## Rationale

- **One process, one program.** `quick-swift` analyzes, runs, and exits. Leaked
  analyses are bounded by the program's own size (root + a fragment per distinct
  interpolation evaluated) and reclaimed by process exit. There is no
  long-running host accumulating leaks.
- **Uniform lifetime.** With everything `'static`, root nodes and fragment nodes
  share one type, so interpolation fragments compose with the rest of the
  evaluator for free — no lifetime plumbing, no parallel "fragment node" path.
- **Simplicity over reclamation.** An arena/generational scheme to free fragment
  analyses would add machinery for memory we do not need to reclaim in a
  run-once CLI.

## Consequences

- Memory for analyses is not freed during a run. **Acceptable** for the
  run-once CLI; **revisit** if quick-swift is ever embedded as a library or a
  long-running/REPL host, where fragment analyses would need an arena tied to
  evaluation scope (or a cache keyed by fragment source).
- New evaluator code should assume `'static` nodes. Do not thread a borrow
  lifetime back through the interpreter — it will collide with interpolation.
- Any feature that re-analyzes source at runtime (future `eval`-like constructs)
  follows the same leak-and-reuse pattern.
