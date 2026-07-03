import Foundation

// ── subscript get ────────────────────────────────────────────────────────────
var d = Data([10, 20, 30, 40])
print(d[0])      // 10
print(d[3])      // 40

// ── subscript set ────────────────────────────────────────────────────────────
d[1] = 99
print(d[1])      // 99

// ── for-in (makeIterator semantics) ─────────────────────────────────────────
var sum = 0
for byte in d {
    sum += Int(byte)
}
print(sum)       // 10 + 99 + 30 + 40 = 179

// ── replaceSubrange(_:with:) ─────────────────────────────────────────────────
var r = Data([1, 2, 3, 4, 5])
r.replaceSubrange(1..<3, with: Data([20, 30, 40]))
// original: [1,2,3,4,5]; replace indices 1..<3 → [1, 20,30,40, 4, 5]
print(r.count)   // 6
print(r[1])      // 20
print(r[4])      // 4

// ── reserveCapacity (no-op) ──────────────────────────────────────────────────
var rc = Data([1, 2, 3])
rc.reserveCapacity(100)
print(rc.count)  // 3

// ── resetBytes(in:) ──────────────────────────────────────────────────────────
var rb = Data([1, 2, 3, 4])
rb.resetBytes(in: 1..<3)
print(rb[0])     // 1
print(rb[1])     // 0
print(rb[2])     // 0
print(rb[3])     // 4

// ── range(of:) found ─────────────────────────────────────────────────────────
let h = Data([1, 2, 3, 4, 5])
let found = h.range(of: Data([3, 4]))!
print(found.lowerBound)  // 2
print(found.upperBound)  // 4

// ── range(of:) absent ────────────────────────────────────────────────────────
print(h.range(of: Data([6, 7])) == nil)   // true

// ── range(of:) empty needle → nil ───────────────────────────────────────────
print(h.range(of: Data([])) == nil)       // true

// ── index(after:) / index(before:) ──────────────────────────────────────────
print(d.index(after: 0))   // 1
print(d.index(before: 3))  // 2
