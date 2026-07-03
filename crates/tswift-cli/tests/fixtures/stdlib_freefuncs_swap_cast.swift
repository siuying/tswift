// Free functions: swap(_:_:) and numericCast(_:).

// ---- swap -------------------------------------------------------------------
// Basic swap of two Int variables.
var x = 10
var y = 20
swap(&x, &y)
print(x, y)       // 20 10

// Swap struct fields via inout.
var s = "hello"
var t = "world"
swap(&s, &t)
print(s, t)       // world hello

// ---- numericCast ------------------------------------------------------------
// numericCast is an Int-family conversion that preserves the raw integer value.
// This runtime models all integers as Int64 by default, so numericCast is
// effectively an identity cast.
let n: Int = 255
let m = numericCast(n) as Int
print(m)           // 255

let neg: Int = -1
print(numericCast(neg) as Int)  // -1
