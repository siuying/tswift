// expected-no-diagnostics
// Tier 10d/S1 — free utilities & output surface.

print(max(3, 7), min(3, 7))
let m = abs(-5)
var a = 1
var b = 2
swap(&a, &b)

for x in stride(from: 0, to: 10, by: 3) {
    _ = x
}
let evens = Array(stride(from: 1, through: 5, by: 2))
let pairs = Array(zip([1, 2, 3], [10, 20, 30]))
let repeated = Array(repeatElement(9, count: 3))
let powers = Array(sequence(first: 1) { $0 < 16 ? $0 * 2 : nil })

print("a", "b", separator: "-", terminator: "\n")
debugPrint("quoted", 42)
dump([1, 2, 3])

assert(2 + 2 == 4)
precondition(true)

let _ = (m, a, b, evens, pairs, repeated, powers)
