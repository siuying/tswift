// Slice 23 — Array / ArraySlice / ContiguousArray edge-case hardening.
// Ported from Apple's test/stdlib/ArraySlice.swift and
// validation-test/stdlib/Arrays.swift.gyb (deterministic assertions only).

// ── 1. Empty Array ──────────────────────────────────────────────────────────
var emptyArr: [Int] = []
print(emptyArr.count)       // 0
print(emptyArr.isEmpty)     // true

// ── 2. insert at endIndex ───────────────────────────────────────────────────
var c = [1, 2, 3]
c.insert(99, at: c.endIndex)
print(c)                    // [1, 2, 3, 99]

// ── 3. replaceSubrange shrink ───────────────────────────────────────────────
var d = [1, 2, 3, 4, 5]
d.replaceSubrange(1..<3, with: [])
print(d)                    // [1, 4, 5]

// ── 4. replaceSubrange grow ─────────────────────────────────────────────────
var e = [1, 2, 3]
e.replaceSubrange(1..<2, with: [10, 20, 30])
print(e)                    // [1, 10, 20, 30, 3]

// ── 5. removeFirst(n) removes n elements ────────────────────────────────────
var f = [1, 2, 3, 4, 5]
f.removeFirst(2)
print(f)                    // [3, 4, 5]

// ── 6. removeLast(n) removes n elements ─────────────────────────────────────
var g = [1, 2, 3, 4, 5]
g.removeLast(2)
print(g)                    // [1, 2, 3]

// ── 7. removeAll(keepingCapacity:) empties array ────────────────────────────
var h = [1, 2, 3]
h.removeAll(keepingCapacity: true)
print(h.count)              // 0
print(h.isEmpty)            // true

// ── 8. ArraySlice — empty slice (i..<i) has zero count ──────────────────────
let base = [1, 2, 3, 4, 5]
let emptySlice = base[2..<2]
print(emptySlice.isEmpty)   // true
print(emptySlice.count)     // 0
print(Array(emptySlice))    // []

// ── 9. ArraySlice — removeFirst() from non-zero-start slice ─────────────────
var arr9 = [1, 2, 3, 4, 5]
var sl9 = arr9[1..<4]       // [2, 3, 4], startIndex=1
let rf9 = sl9.removeFirst()
print(rf9)                  // 2
print(sl9.count)            // 2
print(sl9[sl9.startIndex])  // 3

// ── 10. ArraySlice — replaceSubrange in slice uses base coords ───────────────
var arr10 = [0, 1, 2, 3, 4]
var sl10 = arr10[1..<4]     // [1, 2, 3], base indices 1..4
sl10.replaceSubrange(sl10.startIndex..<sl10.endIndex, with: [10, 20])
print(sl10.count)           // 2
print(sl10[sl10.startIndex])       // 10
print(sl10[sl10.startIndex + 1])   // 20

// ── 11. ArraySlice — original array unaffected after mutation ────────────────
var arr11 = [1, 2, 3, 4, 5]
var sl11 = arr11[1..<4]
sl11.append(99)
print(arr11[1])             // 2  (original unaffected)
print(arr11.count)          // 5

// ── 12. ContiguousArray — basic slice gives view ─────────────────────────────
let ca: ContiguousArray<Int> = [10, 20, 30, 40]
let casl = ca[1..<3]
print(casl.count)           // 2
print(casl[1])              // 20
print(Array(casl))          // [20, 30]

// ── 13. Array description renders elements ────────────────────────────────────
print([1, 2, 3].description)          // [1, 2, 3]
print(["a", "b"].description)         // ["a", "b"]
print([[1, 2], [3]].description)      // [[1, 2], [3]]
