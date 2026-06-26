// expected-no-diagnostics
// Tier 10c/S5 — Sequence/Collection algorithm layer.

let nums = [3, 1, 4, 1, 5, 9, 2, 6]
let doubled = nums.map { $0 * 2 }
let evens = nums.filter { $0 % 2 == 0 }
let sum = nums.reduce(0) { $0 + $1 }
let big = nums.compactMap { $0 > 3 ? $0 : nil }
let flat = [[1, 2], [3]].flatMap { $0 }
let has = nums.contains(5)
let all = nums.allSatisfy { $0 > 0 }
let firstBig = nums.first(where: { $0 > 4 })
let idx = nums.firstIndex(of: 4)
let n = nums.count(where: { $0 > 3 })
let asc = nums.sorted()
let desc = nums.sorted(by: { $0 > $1 })
let lo = nums.min()
let hi = nums.max()
let rev = Array(nums.reversed())
let pre = nums.prefix(3)
let suf = nums.suffix(2)
let df = nums.dropFirst()
let dl = nums.dropLast(2)
let parts = [1, 2, 0, 3].split(separator: 0)
let s = ["a", "b"].joined(separator: "-")
let eq = [1, 2].elementsEqual([1, 2])
let sw = [1, 2, 3].starts(with: [1, 2])
let squares = (1...5).map { $0 * $0 }

let _ = (doubled, evens, sum, big, flat, has, all, firstBig, idx, n,
         asc, desc, lo, hi, rev, pre, suf, df, dl, parts, s, eq, sw, squares)
