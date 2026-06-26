# ADR-0006: String.Index as a UTF-8 byte offset, and the encoding views

- **Status:** Accepted
- **Date:** 2026-06-26
- **Context slice:** Standard library — String indexing (issues #106 design, #107 impl)
- **Builds on:** `docs/plan/stdlib-support.md §7.3 (N3)`; the existing grapheme
  segmenter in `crates/qswift-std/src/string.rs`

## Context

The runtime stores a `String` as a plain Rust `String` (UTF-8 bytes) and
navigates it by **extended grapheme cluster** via a hand-rolled UAX-#29 subset
(`graphemes()`). There is no `String.Index`, and no `unicodeScalars` / `utf8` /
`utf16` views.

N3 (#107) needs `String.Index` + navigation (`startIndex`/`endIndex`/
`index(_:offsetBy:)`/`distance`), the three encoding views, and index-based
mutation (`insert`/`remove`/`replaceSubrange`/`removeSubrange`). The
**acceptance bar is 100% behavioural compatibility with swiftc 6.3.2.** This ADR
records the representation decision so the implementation is mechanical.

## Decision

### D1 — `String.Index` is a UTF-8 byte offset (`encodedOffset`)

A `String.Index` is encoded as a **byte offset into the backing UTF-8**, exactly
like Swift's real index. This is the only representation under which `String`,
`Character`, and the `unicodeScalars`/`utf8`/`utf16` views can **share one index
space** (in Swift they are all literally `String.Index`).

Rejected: a grapheme-cluster ordinal — it cannot interoperate with the encoding
views and re-segments O(n) per use.

### D2 — Dedicated value variant carrying a transcoded sub-offset

`String.Index` is a **first-class `SwiftValue` variant**, not a reused struct:

```
SwiftValue::StringIndex { utf8: usize, transcoded: u32 }
```

- `utf8` is the byte offset; `transcoded` is the sub-position **within a single
  scalar** for the UTF-16 view (0 or 1, distinguishing the two surrogate halves
  of an astral scalar — the one place a pure byte offset is insufficient).
  `transcoded` is `0` for all String/Character/unicodeScalars/utf8 indices.
- `Comparable` and `Hashable` derive from the `(utf8, transcoded)` pair.
  `endIndex` is `{ utf8: bytes.len(), transcoded: 0 }`.

Carrying `transcoded` from day one avoids retrofitting a shipped enum variant.
Rejected: a tagged `Struct` — it would need type-name special-casing for
`Comparable`/print/hash anyway and leaks an inspectable fake type.

### D3 — Element subscript addresses by suffix segmentation

`s[i]` returns the **first grapheme cluster of the suffix beginning at byte
`i.utf8`** (segment `bytes[i.utf8..]`, take cluster 0). For a grapheme-aligned
`i` this is the expected `Character`; for a mid-cluster `i` (from a view) it is
the cluster starting there — both matching swiftc. `i.utf8 == endIndex` (empty
suffix) **traps** with an out-of-bounds message.

### D4 — Trap semantics match swiftc exactly

- element subscript at `endIndex` → trap;
- `index(after:)` / `index(_:offsetBy:)` advancing past `endIndex`, or before
  `startIndex`, without a limit → trap;
- `index(_:offsetBy:limitedBy:)` returns `nil` instead of trapping at the limit;
- `samePosition(in:)` returns `nil` for a non-grapheme-aligned index.

### D5 — Encoding views are backed view values sharing the index space

`s.unicodeScalars` / `s.utf8` / `s.utf16` evaluate to a **backed view value**
(`SwiftValue::StringView { base, kind }`) over the same UTF-8 backing, **not** a
materialized `Array`. Each view is a `Collection` indexed by the shared
`String.Index`:

- `utf8`: element `UInt8` (`Int` width `U8`); `index(after:)` advances one byte;
- `utf16`: element `UInt16` (`Int` width `U16`); advances one UTF-16 code unit
  (stepping `transcoded` across surrogate pairs of astral scalars);
- `unicodeScalars`: element `Unicode.Scalar`; advances one scalar.

`.count` / `.first` / `.last` / iteration / `String.Index` subscripting all work
off the backing. Array materialization was rejected because it cannot subscript
by `String.Index` nor share the index space (#107's explicit requirement).

### D6 — `Unicode.Scalar` is a single-scalar string-like value

`Unicode.Scalar` is modelled as a one-scalar string value exposing `.value`
(`UInt32` code point), mirroring the existing `Character`-as-single-grapheme
model. It prints as the character, like Swift.

### D7 — Reuse the existing segmenter; index print is opaque

Grapheme navigation reuses the hand-rolled `graphemes()` (offline-build
constraint — no new crate). `String.Index` has **no stable printed form**
(Swift's is version-unstable too); fixtures never assert it directly.

### D8 — Seam placement

`SwiftValue::StringIndex` / `StringView`, their `Comparable`/`Hashable`/subscript
behaviour, and grapheme suffix-addressing live in **core** (`value.rs`,
`interp.rs`). The method surface (`index`, `index(after:)`, `distance`,
`startIndex`/`endIndex`, the view accessors, and the mutation methods) is
registered through the **`qswift-std` dispatch seam**.

## Consequences

- Two new `SwiftValue` variants (`StringIndex`, `StringView`) touch the
  exhaustive match sites (`Display`, `values_equal`, `value_less_than`,
  `type_name`, subscripting) — the bulk of #107's core surface.
- Views are lazy and faithful: `s.utf8[s.startIndex]` and cross-view index use
  behave as in Swift, including astral surrogate halves via `transcoded`.
- Mid-cluster subscripting and `samePosition` round/validate by re-segmenting the
  relevant suffix — correct but O(n) per call; acceptable for a tree-walker.
- `String.Index` is usable as a `Dictionary`/`Set` key (Hashable from the pair).

## Scope for #107

Deliver, verified byte-for-byte against swiftc 6.3.2:
`startIndex`/`endIndex`/`index(after:)`/`index(before:)`/`index(_:offsetBy:)`(`limitedBy:`)/
`distance(from:to:)`, element subscript and `Range` subscript, the three views
with shared-index subscripting and `count`/`first`/iteration, and
`insert(_:at:)`/`insert(contentsOf:at:)`/`remove(at:)`/`removeSubrange`/
`replaceSubrange` with copy-on-write correctness.
