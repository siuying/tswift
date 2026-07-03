// Slice 23 — Set edge-case hardening.
// Ported from Apple's validation-test/stdlib/Set.swift and SetAlgebra.swift
// (deterministic assertions, no ObjC/bridge/perf tests).

// ── 1. Empty set operations ───────────────────────────────────────────────────
var empty: Set<Int> = []
print(empty.isEmpty)                         // true
print(empty.union([]).isEmpty)               // true
print(empty.intersection([1, 2, 3]).isEmpty) // true
print(empty.subtracting([1, 2]).isEmpty)     // true

// ── 2. Self operations (identity laws) ────────────────────────────────────────
let s: Set<Int> = [1, 2, 3]
print(s.union(s).sorted())               // [1, 2, 3]
print(s.intersection(s).sorted())        // [1, 2, 3]
print(s.subtracting(s).isEmpty)          // true
print(s.symmetricDifference(s).isEmpty)  // true

// ── 3. insert — return value (inserted:memberAfterInsert:) ────────────────────
var s3: Set<Int> = [1, 2, 3]
let (ins1, mem1) = s3.insert(4)
print(ins1)   // true  (new element)
print(mem1)   // 4
let (ins2, mem2) = s3.insert(1)
print(ins2)   // false (already present)
print(mem2)   // 1

// ── 4. isSubset / isSuperset edge cases (empty and reflexive) ─────────────────
let a: Set<Int> = []
let b: Set<Int> = [1, 2]
print(a.isSubset(of: b))     // true  (empty is subset of everything)
print(a.isSubset(of: a))     // true  (reflexive)
print(b.isSuperset(of: a))   // true  (everything is superset of empty)
print(b.isSuperset(of: b))   // true  (reflexive)

// ── 5. isDisjoint ────────────────────────────────────────────────────────────
let c: Set<Int> = [1, 2]
let d: Set<Int> = [3, 4]
let e: Set<Int> = [2, 3]
print(c.isDisjoint(with: d))  // true  (no shared elements)
print(c.isDisjoint(with: e))  // false (2 is shared)
print(a.isDisjoint(with: b))  // true  (empty is disjoint with anything)

// ── 6. isStrictSubset / isStrictSuperset ─────────────────────────────────────
let x: Set<Int> = [1]
let y: Set<Int> = [1, 2]
print(x.isStrictSubset(of: y))   // true
print(y.isStrictSubset(of: y))   // false (equal, not strict)
print(y.isStrictSuperset(of: x)) // true
print(y.isStrictSuperset(of: y)) // false (equal, not strict)

// ── 7. symmetricDifference ────────────────────────────────────────────────────
var f: Set<Int> = [1, 2, 3]
print(f.symmetricDifference([2, 3, 4]).sorted())  // [1, 4]

// ── 8. for-in over set ───────────────────────────────────────────────────────
var total = 0
for n in Set<Int>([3, 1, 2]) { total += n }
print(total)   // 6

// ── 9. isEmpty after removeAll ────────────────────────────────────────────────
var g: Set<Int> = [1, 2, 3]
g.removeAll()
print(g.isEmpty)   // true
print(g.count)     // 0

// ── 10. update(with:) — returns displaced value ───────────────────────────────
var h: Set<Int> = [10, 20, 30]
let prev = h.update(with: 20)
print(prev!)       // 20 (old member replaced)
print(h.count)     // 3  (count unchanged)

// ── 11. subtract and formSymmetricDifference ──────────────────────────────────
var i: Set<Int> = [1, 2, 3, 4]
i.subtract([2, 4])
print(i.sorted())  // [1, 3]

var j: Set<Int> = [1, 2, 3]
j.formSymmetricDifference([3, 4, 5])
print(j.sorted())  // [1, 2, 4, 5]
