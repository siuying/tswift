// Slice 26 — Set and Dictionary index-based members.
// All fixtures use single-element collections or element-level assertions to
// remain deterministic regardless of internal storage order.

// ── 1. Set.startIndex / endIndex ─────────────────────────────────────────────
var s1: Set<Int> = [42]
let si1 = s1.startIndex
let ei1 = s1.endIndex
print(si1 == ei1)  // false — non-empty set has distinct start/end
print(si1 == s1.startIndex)  // true — stable

// ── 2. Set.subscript(Index) ───────────────────────────────────────────────────
print(s1[si1])  // 42

// ── 3. Set.index(after:) ─────────────────────────────────────────────────────
let ai1 = s1.index(after: si1)
print(ai1 == ei1)  // true — one-element set: after(start) == end

// ── 4. Set.firstIndex(of:) → Set.Index? ───────────────────────────────────────
let found = s1.firstIndex(of: 42)
print(found != nil)   // true
let notFound = s1.firstIndex(of: 0)
print(notFound == nil)  // true — absent element

// ── 5. Set.firstIndex(of:) subscript round-trip ───────────────────────────────
if let idx = s1.firstIndex(of: 42) {
    print(s1[idx])  // 42
}

// ── 6. Set.remove(at:) ───────────────────────────────────────────────────────
var s2: Set<Int> = [99]
let ri = s2.startIndex
let removed = s2.remove(at: ri)
print(removed)       // 99
print(s2.isEmpty)    // true

// ── 7. Set.makeIterator / next ────────────────────────────────────────────────
var s3: Set<Int> = [7]
var it3 = s3.makeIterator()
print(it3.next()!)   // 7
print(it3.next() == nil)  // true — exhausted

// ── 8. Set.startIndex == endIndex for empty set ───────────────────────────────
let empty: Set<Int> = []
print(empty.startIndex == empty.endIndex)  // true

// ── 9. Set.remove(at:) with multi-element — remove first ─────────────────────
var s4: Set<Int> = [10]
_ = s4.insert(20)
_ = s4.insert(30)
let firstElem = s4[s4.startIndex]
s4.remove(at: s4.startIndex)
print(s4.count)  // 2
print(s4.contains(firstElem))  // false — it was removed

// ── 10. Dictionary.startIndex / endIndex ──────────────────────────────────────
var d1: [String: Int] = ["x": 5]
let di1 = d1.startIndex
let dei1 = d1.endIndex
print(di1 == dei1)  // false
print(di1 == d1.startIndex)  // true

// ── 11. Dictionary.subscript(Index) → labeled tuple ──────────────────────────
print(d1[di1])  // (key: "x", value: 5)

// ── 12. Dictionary.index(after:) ─────────────────────────────────────────────
let dai1 = d1.index(after: di1)
print(dai1 == dei1)  // true

// ── 13. Dictionary.index(forKey:) ────────────────────────────────────────────
let kIdx = d1.index(forKey: "x")
print(kIdx != nil)   // true
let kAbsent = d1.index(forKey: "z")
print(kAbsent == nil)  // true

// ── 14. Dictionary.subscript(index(forKey:)) round-trip ─────────────────────
if let ki = d1.index(forKey: "x") {
    print(d1[ki])  // (key: "x", value: 5)
}

// ── 15. Dictionary.remove(at:) → (key:, value:) ──────────────────────────────
var d2: [String: Int] = ["k": 9]
let dri = d2.startIndex
let pair = d2.remove(at: dri)
print(pair)       // (key: "k", value: 9)
print(d2.isEmpty) // true

// ── 16. Dictionary.makeIterator / next ───────────────────────────────────────
var d3: [String: Int] = ["m": 3]
var it4 = d3.makeIterator()
print(it4.next()!)   // (key: "m", value: 3)
print(it4.next() == nil)  // true — exhausted

// ── 17. Dictionary.startIndex == endIndex for empty ─────────────────────────
let emptyD: [String: Int] = [:]
print(emptyD.startIndex == emptyD.endIndex)  // true

// ── 18. Set.formIndex(after:) mutates the index in-place ─────────────────────
var s5: Set<Int> = [55]
var fi5 = s5.startIndex
print(s5[fi5])              // 55
s5.formIndex(after: &fi5)
print(fi5 == s5.endIndex)   // true — fi5 was advanced to endIndex

// ── 19. Dictionary.formIndex(after:) mutates the index in-place ──────────────
var d4: [String: Int] = ["z": 7]
var fi4 = d4.startIndex
print(d4[fi4])              // (key: "z", value: 7)
d4.formIndex(after: &fi4)
print(fi4 == d4.endIndex)   // true — fi4 was advanced to endIndex
