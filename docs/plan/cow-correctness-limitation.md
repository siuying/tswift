# Known limitation: `isKnownUniquelyReferenced` and struct-embedded CoW

**Status:** documented gap (not human-blocked; deferred as an architectural change).

## What works

`isKnownUniquelyReferenced(_:)` is implemented in
`crates/tswift-core/src/interp.rs` (search `isKnownUniquelyReferenced`). It
returns the correct answer for the canonical top-level case:

```swift
final class Box { var n = 0 }
var box = Box()
print(isKnownUniquelyReferenced(&box)) // true
let other = box
print(isKnownUniquelyReferenced(&box)) // false
```

This is covered by `crates/tswift-cli/tests/fixtures/stdlib_s1_utilities.swift`.

## What does not work

The idiomatic copy-on-write *buffer* pattern — a `struct` wrapping a `class`
storage object — returns the wrong answer:

```swift
struct Buffer {
    private var storage = Box(0)
    mutating func write(_ v: Int) {
        if !isKnownUniquelyReferenced(&storage) {   // wrongly reads `true`
            storage = Box(storage.value)            // copy is skipped
        }
        storage.value = v
    }
    var value: Int { storage.value }
}
var x = Buffer(); var y = x
x.write(5)
print(x.value, y.value) // prints "5 5"; Swift prints "5 0"
```

## Root cause

Struct values are modelled as `SwiftValue::Struct(Rc<StructObj>)` with **lazy
`Rc::make_mut` copy-on-write** at the struct level. When `var y = x` copies a
struct, the runtime clones only the *outer* `Rc<StructObj>` — it does **not**
eagerly retain the struct's reference-type fields. So the inner `Box`'s strong
count stays `1` even though two logical struct values embed it.

`isKnownUniquelyReferenced` decides uniqueness from the `Rc` strong count of the
class instance (`== 2`, accounting for the env binding + the evaluated argument
clone). Because struct copies don't bump the inner class refcount, a shared
storage object reads as unique.

## Why it is deferred

A correct fix is an architecture-level change, not a local patch. Options:

1. **Eager field retain on struct copy** — abandon lazy struct CoW so a struct
   assignment retains each reference-type field. Simple to reason about, but a
   broad perf/semantics change touching every struct copy site.
2. **Unique-at-mutating-entry** — make a struct's `StructObj` unique when a
   `mutating` method begins, so field access goes through a uniquely-owned copy
   whose class fields were retained by the clone. Narrower, but the transient
   `Rc` clones introduced by the call mechanism (the `self` binding, argument
   evaluation) make the `== 2` heuristic brittle and hard to calibrate across
   all dispatch paths.

Either path needs its own design pass and a dedicated regression-fixture set.
Until then the canonical top-level use is supported; the struct-buffer idiom is
a known gap.
