// Slice 23 — Range / ClosedRange edge-case hardening.
// Ported from Apple's test/stdlib/RangeTraps.swift and
// validation-test/stdlib/Range.swift.gyb (deterministic assertions only).

// ── 1. Empty Range (a..<a) ───────────────────────────────────────────────────
let empty = 5..<5
print(empty.isEmpty)       // true
print(empty.count)         // 0
print(empty.contains(5))   // false (exclusive upper)
print(empty.contains(4))   // false

// ── 2. Range contains / overlaps ─────────────────────────────────────────────
let r = 0..<10
print(r.contains(0))       // true
print(r.contains(9))       // true
print(r.contains(10))      // false (exclusive upper)

// exclusive touching: 0..<5 and 5..<10 do NOT overlap
print((0..<5).overlaps(5..<10))   // false
print((0..<5).overlaps(4..<10))   // true (4 is shared)

// ── 3. Range.clamped edges ───────────────────────────────────────────────────
// clamped to partial overlap
print((3..<7).clamped(to: 0..<5))   // 3..<5
print((3..<7).clamped(to: 5..<10))  // 5..<7
// clamped to non-overlapping → empty range at limit
print((3..<7).clamped(to: 8..<12))  // 8..<8

// ── 4. Range single element ───────────────────────────────────────────────────
let one = 5..<6
print(one.count)           // 1
print(one.contains(5))     // true
print(one.contains(6))     // false

// ── 5. Range equality and hash consistency ────────────────────────────────────
print((1..<5) == (1..<5))  // true
print((1..<5) == (1..<6))  // false
print((1..<5).hashValue == (1..<5).hashValue)  // true

// ── 6. ClosedRange single element (0...0) ────────────────────────────────────
let s0 = 0...0
print(s0.count)            // 1
print(s0.isEmpty)          // false
print(s0.contains(0))      // true
print(s0.contains(1))      // false

// ── 7. ClosedRange with negative bounds ──────────────────────────────────────
let neg = -5 ... -1
print(neg.count)           // 5
print(neg.contains(-5))    // true
print(neg.contains(-1))    // true
print(neg.contains(0))     // false

// ── 8. ClosedRange.clamped edges ─────────────────────────────────────────────
// clamped to smaller interior
print((1...10).clamped(to: 3...7))    // 3...7
// clamped to entirely above → lowerBound of limit (11...11)
print((1...10).clamped(to: 11...20))  // 11...11

// ── 9. ClosedRange overlaps ───────────────────────────────────────────────────
print((1...5).overlaps(5...10))   // true (5 shared)
print((1...4).overlaps(5...10))   // false
print((1...5).overlaps(3...3))    // true (3 inside)

// ── 10. ClosedRange — 0...Int.max boundary ───────────────────────────────────
print((0...Int.max).contains(Int.max))  // true

// ── 11. ClosedRange for-in with negative range ────────────────────────────────
var sum = 0
for i in -2...2 { sum += i }
print(sum)   // 0  (-2 + -1 + 0 + 1 + 2)

// ── 12. ClosedRange equality and hash ─────────────────────────────────────────
print((1...5) == (1...5))  // true
print((1...5) == (1...6))  // false
print((3...7).hashValue == (3...7).hashValue)  // true
print((3...7).hashValue == (3...8).hashValue)  // false

// ── 13. Range overlaps ClosedRange sharing a boundary ─────────────────────────
print((0...5).overlaps(5..<10))   // true (5 shared)
