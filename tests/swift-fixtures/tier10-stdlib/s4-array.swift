// expected-no-diagnostics
// Tier 10b/S4 — Array intrinsics + copy-on-write.

var a = [1, 2, 3]
a.insert(0, at: 0)
let removed = a.remove(at: 2)
let popped = a.removeLast()
let front = a.removeFirst()
a.removeAll()
a.reserveCapacity(8)

var b = [10, 20, 30]
b[1] = 99
let c = b.count
let f = b.first
let l = b.last
let s = b.startIndex
let e = b.endIndex

let joined = [1, 2] + [3, 4]
var d = [1]
d += [2, 3]
let repeated = Array(repeating: 0, count: 3)
let fromRange = Array(1...4)

let _ = (removed, popped, front, c, f, l, s, e, joined, d, repeated, fromRange)
