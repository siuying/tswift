# Plan — Standard Library Support

**Status:** proposed
**Date:** 2026-06-26
**Reference toolchain:** Swift **6.3.2** (`swift-6.3.2-RELEASE`)
**Related:**
- `docs/swift-runtime/feature-checklist.md` — Tier 10 is the hand-written surface
- `docs/swift-runtime/stdlib-inventory.md` — generated API inventory (companion)
- `docs/plan/swift-runtime-implementation-plan.md` — overall phasing (§3.2, §5)
- `tools/stdlib-inventory/extract.py` — inventory generator

## 1. Problem statement

The standard library is the largest sustained effort in the runtime
(feature-checklist Tier 10, risk register: *"Stdlib is unbounded — Highest"*).
Today it is implemented **ad hoc and scattered**:

- `crates/qswift-std/src/lib.rs` is a near-empty skeleton — it registers only
  `print`.
- Real behaviour lives as hardcoded `match` arms inside
  `crates/qswift-core/src/interp.rs` (~4900 lines): `array_higher_order`
  (`map`/`filter`/`reduce`/`forEach`/`contains`/`first`/`sorted`), `.count`/
  `.isEmpty`, `max`/`min`/`abs`, `Int("…")` conversions, `Result.get()`, JSON
  coders, and more.
- Tier 10 of the checklist is **entirely unchecked**, even though fixtures in
  `tests/swift-fixtures/tier10-stdlib/` already exercise a slice of it — there is
  no systematic map from "what Swift provides" to "what we implement" to "what is
  verified."

This plan establishes (1) how we enumerate the stdlib surface, (2) the reference
we measure against, and (3) how we implement and track it.

## 2. Goal 1 — Enumerate the stdlib surface (machine-generated)

We do **not** hand-curate the API list. We extract it from the reference
toolchain's `.swiftinterface`, which is the exact public surface of the shipped
stdlib.

- **Source:** `…/swift-6.3.2-RELEASE.xctoolchain/usr/lib/swift/macosx/Swift.swiftmodule/arm64-apple-macos.swiftinterface`
  (≈53.6k lines, valid Swift, full signatures + generic constraints).
- **Tool:** `tools/stdlib-inventory/extract.py` — a brace-depth surface extractor
  that groups every public `func`/`var`/`subscript`/`init`/`case` under its owning
  type or `extension`, filtering out underscore-prefixed runtime internals and
  ObjC-bridging shims.
- **Output:** `docs/swift-runtime/stdlib-inventory.md` (regenerated, never edited
  by hand): **192 types, 99 free functions**.

Regenerate with:

```sh
F=~/Library/Developer/Toolchains/swift-6.3.2-RELEASE.xctoolchain/usr/lib/swift/macosx/Swift.swiftmodule/arm64-apple-macos.swiftinterface
python3 tools/stdlib-inventory/extract.py "$F" > docs/swift-runtime/stdlib-inventory.md
```

The inventory is the **complete companion** to the curated Tier 10 checklist: the
checklist says *what we intend to support and in which phase*; the inventory
guarantees we never silently drift from the reference surface.

## 3. Goal 2 — Reference version

- **Pinned to Swift 6.3.2** (installed via `swiftly`; repo-local `.swift-version`
  records the pin so differential testing uses the same toolchain).
- Two reference roles:
  - **API shape** — the `.swiftinterface` above (signatures, constraints).
  - **Behaviour / ground truth** — running fixtures through real `swiftc` 6.3.2 and
    diffing stdout (the plan's §5 differential-testing strategy). This is how we
    decide "exact Swift behaviour" for floats, ordering, error messages, etc.
- **Unicode pin:** `unicode-segmentation`/`-normalization` must track the Unicode
  version of 6.3.2 (see overall plan §3.3); a CI fixture catches drift.

## 4. Goal 3 — Implementation plan

### 4.1 Refactor: give the stdlib a real home (incremental)

Lift the scattered behaviour out of `interp.rs` into `qswift-std` behind a clean
native-dispatch seam — **incrementally, one receiver type at a time** (`Array`
first, then `String`, `Dictionary`, `Set`, numerics, `Optional`). Each step is a
pure relocation guarded by the existing fixtures; no behaviour change per step.

**Registry design — two layers (the recommended hybrid).** The inventory shows
the higher-order algorithms (`map`/`filter`/`reduce`/`sorted`/`contains`/
`prefix`/`enumerated`…) live on `Sequence`/`Collection`, *not* on `Array`
directly — so they must be written **once**, not per concrete type. That rules out
a purely flat registry and motivates two layers:

1. **Intrinsic registry** — `HashMap<(BuiltinReceiver, &str), NativeMethodFn>`
   for *type-specific* members: `Array.append`/`insert`/`removeAll`/`count`,
   `Dictionary.keys`/`values`/`updateValue`, `String.uppercased`/`hasPrefix`,
   `Int.isMultiple`/`signum`, `Set.union`/`intersection`, etc. `BuiltinReceiver`
   is an enum (`Array`/`Dictionary`/`Set`/`String`/`Int`/`Double`/`Optional`/…).
   Simple, fast, trivially unit-testable.
2. **Protocol-algorithm layer** — the `Sequence`/`Collection` algorithms written
   once against a small `as_sequence(&SwiftValue) -> Option<impl Iterator<
   Item = SwiftValue>>` adapter, applied to *any* builtin sequence receiver.
   This is the seed of real `Sequence`/`Collection` witness modelling (Tier 10c)
   and removes the per-type duplication the current `array_higher_order` has.

**Considered and rejected:**
- *Flat tuple registry only* — forces `map`/`filter`/… to be re-registered on
  every concrete sequence type (Array, Set, Dictionary, Range, String views).
  Duplication; diverges from Swift's protocol-extension model.
- *Per-type trait objects only* (`trait BuiltinType { fn call(…) }`) — groups
  intrinsics nicely but still duplicates the sequence algorithms per type.
- *Full protocol-witness tables for everything* — most faithful, but pulls the
  whole conformance machinery forward before we need it; deferred, with layer 2
  as the incremental on-ramp.

**Dispatch order for a builtin receiver** (mirrors Swift, pragmatic):
(1) intrinsic registry → (2) protocol-algorithm layer → (3) user `extension`
methods on the builtin type → (4) protocol default/witness. We do **not** perform
full overload resolution, so a user overload that reuses a stdlib method name does
not shadow the builtin — a documented limitation.

This is itself a deep-module exercise (see the `codebase-design` skill): one
narrow registration seam, behaviour hidden behind it.

### 4.2 Coverage tracking (from both registry and fixtures)

`tools/stdlib-inventory/coverage.py` cross-references the generated inventory
against **two** signals and assigns each member a state:

- **missing** — not in the `qswift-std` registry.
- **implemented** — present in the registry (declared coverage).
- **verified** — in the registry **and** exercised by a passing fixture
  (behavioural coverage).

The registry signal is read from a generated manifest the registry emits at build
/ test time (so the tool needs no Rust parsing); the fixture signal is read from
the `tier10-stdlib` fixtures + the `golden_fixtures` results. Output is a
coverage report (per-type %, overall %) that feeds Tier 10 checkbox updates —
turning "stdlib is unbounded" into a tracked number with a verified subset.

### 4.3 Ordered implementation list

Concrete, ordered worklist derived from the inventory (member names are the
user-facing public API of Swift 6.3.2). Each numbered step is a vertical slice:
registry entries + fixtures + checklist/coverage update. Steps are demand-driven
but this is the default order. (Scope ceiling: **Foundation is out** — no
`Decimal`/`Date`/`URL`/`Data`; regex literals deferred to their checklist phase.)

**S1 — Free utilities + output (10d core).** `print`/`debugPrint`/`dump`,
`assert`/`assertionFailure`/`precondition`/`preconditionFailure`/`fatalError`,
`min`/`max`/`abs`/`swap`, `zip`/`stride`/`repeatElement`/`sequence`,
`readLine`, `isKnownUniquelyReferenced`. *(Also the refactor of the existing
`print` + numeric helpers into the new seam.)*

**S2 — Core scalar values (10a).** `Int`/`UInt` widths + overflow/wrapping ops +
`isMultiple`/`signum`/`quotientAndRemainder`; `Double`/`Float` math
(`rounded`/`squareRoot`/`isNaN`/`magnitude`/`truncatingRemainder`); `Bool`;
failable string conversions `Int("…")`/`Double("…")`; width conversions.

**S3 — Ranges & Optional (10a).** `Range`/`ClosedRange` (`contains`/`count`/
`lowerBound`/`upperBound`/`clamped`), one-sided ranges; `Optional.map`/`flatMap`,
`??`, pattern hooks already in core — move behaviour into the registry.

**S4 — Array intrinsics + CoW (10b, ★★★★).** `append`/`insert(at:)`/
`remove(at:)`/`removeAll`/`removeLast`/`reserveCapacity`/`+`/`+=`, `count`/
`isEmpty`/`first`/`last`/`startIndex`/`endIndex`/`capacity`, subscript get/set,
`init(repeating:count:)`/`init(_: Sequence)`. Verify copy-on-write via
`Rc::make_mut` uniqueness tests.

**S5 — Sequence/Collection algorithm layer (10c).** Implemented once against the
`as_sequence` adapter, then available to Array/Set/Dictionary/Range/String views:
`map`/`filter`/`reduce`/`compactMap`/`flatMap`/`forEach`/`contains`/`allSatisfy`/
`first(where:)`/`firstIndex`/`sorted`/`sorted(by:)`/`min`/`max`/`reversed`/
`enumerated`/`prefix`/`suffix`/`dropFirst`/`dropLast`/`split`/`joined`/`count`/
`elementsEqual`/`starts(with:)`/`randomElement`/`shuffled`.

**S6 — Dictionary + CoW (10b, ★★★★).** subscript get/set/default,
`keys`/`values`/`count`/`isEmpty`, `updateValue`/`removeValue`/`merge`/`merging`/
`mapValues`/`compactMapValues`, `init(uniqueKeysWithValues:)`/
`init(grouping:by:)`.

**S7 — Set + CoW (10b).** `insert`/`remove`/`contains`/`update`, `union`/
`intersection`/`subtracting`/`symmetricDifference` (+ `form*` mutating),
`isSubset`/`isSuperset`/`isDisjoint`, `count`/`isEmpty`.

**S8 — String / Character / Substring (10a, ★★★★).** `count`/`isEmpty`/`first`/
`last`, `uppercased`/`lowercased`, `hasPrefix`/`hasSuffix`/`contains`, `append`/
`+`/`+=`, `split`/`replacingOccurrences`(if in scope)/`prefix`/`suffix`,
index/subscript over grapheme clusters, `Substring` view, `Character` value,
UTF-8/Unicode-segmentation backing pinned to 6.3.2.

**S9 — Language-driving protocols + conformance synthesis (10c).**
`Equatable`/`Hashable`/`Comparable` (operators + `hash(into:)`),
`CustomStringConvertible`/`CustomDebugStringConvertible` (`description`),
`RawRepresentable`/`CaseIterable`, `ExpressibleBy*Literal`, `Identifiable`,
`Codable` round-trip via the existing JSON layer.

**S10 — Slices & remaining containers (10b).** `ArraySlice`/`ContiguousArray`,
`CollectionOfOne`/`EmptyCollection`, `Result` (move from ad-hoc), `KeyValuePairs`.

`String`/`Character`/`Substring` (S8) and `Array`/`Dictionary` CoW (S4/S6) are the
hardest items (★★★★) — schedule extra time and lean on differential testing
against real `swiftc` 6.3.2.

### 4.4 Definition of done per member

Matches the overall plan §7: the member is dispatched from `qswift-std`, has ≥1
golden fixture in `tests/swift-fixtures/tier10-stdlib/`, the `golden_fixtures`
test passes, output matches real `swiftc` 6.3.2 where applicable, and the Tier 10
checklist + coverage report are updated. Any intentional compatibility gap (float
formatting, regex subset, Foundation exclusions) is documented per §3.3.

## 5. Deliverables

- [x] `tools/stdlib-inventory/extract.py` + generated `stdlib-inventory.md`
- [ ] `.swift-version` pinned to 6.3.2 (committed)
- [ ] Two-layer dispatch seam in `qswift-std` (intrinsic registry + algorithm layer)
- [ ] Incremental refactor of `interp.rs` ad-hoc arms into the seam (Array first)
- [ ] `tools/stdlib-inventory/coverage.py` + three-state coverage report
- [ ] Ordered implementation S1–S10 (§4.3), each a vertical slice with fixtures +
      checklist/coverage updates

## 6. Resolved decisions

- **Refactor blast radius** → **incremental**, one receiver type at a time, each
  step guarded by existing fixtures (§4.1).
- **Registry key design** → **two-layer hybrid**: flat intrinsic registry +
  write-once `Sequence`/`Collection` algorithm layer; dispatch order documented
  (§4.1).
- **Scope ceiling** → Foundation **out** of MVP; an explicit ordered list S1–S10
  (§4.3); regex literals deferred to their checklist phase.
- **Coverage signal** → **both** registry (implemented) and fixtures (verified),
  three-state report (§4.2).
