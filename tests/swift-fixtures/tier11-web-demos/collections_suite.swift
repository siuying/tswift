// expected-no-diagnostics
// Tier 11 / Web demo — Collections: Array, Dictionary, Set — combined showcase.

// ── Array ──
var nums = [3, 1, 4, 1, 5, 9, 2, 6]
nums.sort()
print("sorted:  \(nums)")
print("first:   \(nums.first!), last: \(nums.last!)")

let squares = (1...5).map { $0 * $0 }
print("squares: \(squares)")

let flat = [[1, 2], [3, 4], [5]].flatMap { $0 }
print("flat:    \(flat)")

// ── Dictionary ──
var scores = ["Alice": 95, "Bob": 72, "Carol": 88]
scores["Dave"] = 91
let top = scores.filter { $0.value >= 90 }.keys.sorted()
print("top scorers: \(top)")

let doubled = scores.mapValues { $0 * 2 }
print("doubled Bob: \(doubled["Bob"]!)")

// ── Set ──
let a: Set<Int> = [1, 2, 3, 4, 5]
let b: Set<Int> = [3, 4, 5, 6, 7]
print("union:        \(a.union(b).sorted())")
print("intersection: \(a.intersection(b).sorted())")
print("a − b:        \(a.subtracting(b).sorted())")
print("symmetric Δ:  \(a.symmetricDifference(b).sorted())")
