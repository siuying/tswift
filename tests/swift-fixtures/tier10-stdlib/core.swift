// expected-no-diagnostics
// Tier 10a/10d — core values, Range, stride, and global utility functions.

let i: Int = 42
let u: UInt8 = 255
let d: Double = 3.14
let flag: Bool = true

let bigger = max(i, 100)
let smaller = min(i, 7)
let magnitude = abs(-5)
let wrapped = u &+ 1

let range = 0 ..< 5
let strided = Array(stride(from: 0, to: 10, by: 2))
let parsed: Int? = Int("123")

print("values:", i, u, d, flag)

let _ = (bigger, smaller, magnitude, wrapped, range.count, strided.count, parsed)
