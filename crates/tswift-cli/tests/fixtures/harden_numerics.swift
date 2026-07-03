// harden_numerics.swift — numeric edge cases ported from Apple's Swift test suite
// Ground-truthed against Swift 6.3.2 (2026-07-03)
// Sources: test/stdlib/PrintInteger.swift, PrintFloat*.swift, Integers.swift.gyb, Float.swift

// --- Integer bounds printing ---
print(Int.max)    // 9223372036854775807
print(Int.min)    // -9223372036854775808
print(Int8.max)   // 127
print(Int8.min)   // -128
print(UInt8.max)  // 255
print(UInt8.min)  // 0
print(Int16.max)  // 32767
print(Int16.min)  // -32768
print(UInt16.max) // 65535
print(Int32.max)  // 2147483647
print(Int32.min)  // -2147483648
print(UInt32.max) // 4294967295
print(Int64.max)  // 9223372036854775807
print(Int64.min)  // -9223372036854775808
print(UInt64.max) // 18446744073709551615

// --- Wrapping arithmetic (&+, &-, &*) ---
print(Int.max &+ 1 == Int.min)    // true
print(Int.min &- 1 == Int.max)    // true
print(Int.min &* (-1) == Int.min) // true

let a8: Int8 = 127
print(a8 &+ 1)   // -128  (Int8 wraps)
print(a8 &+ 2)   // -127
let b8: Int8 = -128
print(b8 &- 1)   // 127   (Int8 wraps)
let c8: UInt8 = 255
print(c8 &+ 1)   // 0     (UInt8 wraps)
print(c8 &* 2)   // 254   ((255*2) & 0xFF)

// --- Integer division truncates toward zero ---
print(7 / 2)    // 3
print(-7 / 2)   // -3
print(7 / -2)   // -3
print(-7 / -2)  // 3

// --- Integer remainder: sign follows dividend ---
print(7 % 3)    // 1
print(-7 % 3)   // -1
print(7 % -3)   // 1
print(-7 % -3)  // -1

// --- min / max / abs ---
print(min(3, 5))  // 3
print(max(3, 5))  // 5
print(min(3, -5)) // -5
print(abs(-42))   // 42
print(abs(42))    // 42

// --- Double printing: decimal form ---
print(0.1)          // 0.1
print(0.2)          // 0.2
print(0.1 + 0.2)    // 0.30000000000000004
print(1.0 / 3.0)    // 0.3333333333333333
print(2.0 / 3.0)    // 0.6666666666666666
print(0.0)          // 0.0
print(-0.0)         // -0.0

// --- Double printing: scientific notation for large values (exponent >= 16) ---
print(1e16)   // 1e+16
print(1e21)   // 1e+21
print(1.5e21) // 1.5e+21

// --- Double printing: scientific notation for small values (exponent < -4) ---
print(1e-10)   // 1e-10
print(1.5e-10) // 1.5e-10
print(1e-5)    // 1e-05

// --- Double printing: decimal form at the boundaries ---
print(1e15)  // 1000000000000000.0  (exact integer < 2^53, decimal form)
print(1e-4)  // 0.0001              (exponent == -4, decimal form)

// --- Double printing: special values ---
print(Double.infinity)              // inf
print(-Double.infinity)             // -inf
print(Double.nan)                   // nan
print(Double.greatestFiniteMagnitude) // 1.7976931348623157e+308
print(Double.leastNormalMagnitude)    // 2.2250738585072014e-308
print(Double.leastNonzeroMagnitude)   // 5e-324

// --- NaN comparison semantics (IEEE 754) ---
let nan = Double.nan
print(nan == nan)  // false
print(nan != nan)  // true

// --- -0.0 equals 0.0 ---
print(-0.0 == 0.0) // true

// --- Double.rounded() uses half-away-from-zero ---
print(0.5.rounded())    // 1.0
print(1.5.rounded())    // 2.0
print(2.5.rounded())    // 3.0
print(3.5.rounded())    // 4.0
print((-0.5).rounded()) // -1.0
print((-1.5).rounded()) // -2.0
print((-2.5).rounded()) // -3.0
print((-3.5).rounded()) // -4.0
