# Custom `Sequence` / `IteratorProtocol` — notes & gaps

Status: `for-in` drives a user struct, enum, or class that exposes `next()`
(it is its own iterator) or `makeIterator()`. The iterator is driven lazily,
so an unbounded sequence with `break` terminates; struct/enum iterators write
their mutated state back between iterations, and class iterators mutate through
the reference. Binding names, `for case` patterns, `where`, `break`/`continue`,
and labels all work.

## Notes / gaps

1. **Duck-typed, not conformance-checked.** Eligibility is decided structurally
   by the presence of `next()` / `makeIterator()`, not by a declared
   `Sequence`/`IteratorProtocol` conformance. A value type that happens to have
   such a method would be treated as a sequence.

2. **`next()` preferred over `makeIterator()`.** A type with both is driven as
   its own iterator. This is correct for the common "Sequence & IteratorProtocol"
   pattern (where `makeIterator()` is synthesized to return `self`), but a
   Sequence with an unrelated helper named `next` plus a real `makeIterator`
   would be driven incorrectly.

3. **No default `Sequence` algorithms.** Conforming to `Sequence` does not yet
   synthesize the protocol's default methods (`map`, `filter`, `reduce`,
   `contains`, …) for the user type; only `for-in` iteration is provided.
