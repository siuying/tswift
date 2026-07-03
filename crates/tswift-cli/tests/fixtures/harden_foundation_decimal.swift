import Foundation

// Harden slice 25: Decimal edge cases
// Ground-truth captured from Swift 6.3.2 on macOS.

// --- 38-digit division precision ---
// Previously runtime produced only 30 digits; now 38 (matching NSDecimal).
print(Decimal(1) / Decimal(3))   // 0.33333333333333333333333333333333333333

// --- Basic arithmetic ---
print(Decimal(1) + Decimal(2))   // 3
print(Decimal(10) - Decimal(3))  // 7
print(Decimal(5) * Decimal(3))   // 15

// --- String init ---
print(Decimal(string: "3.14159")!)                       // 3.14159
print(Decimal(string: "invalid") == nil ? "nil" : "ok") // nil

// --- Special value predicates ---
print(Decimal.nan.isNaN)       // true
print(Decimal(0).isZero)       // true
print(Decimal(1).isFinite)     // true
print(Decimal(1).isInfinite)   // false

// --- Sign ---
print(Decimal(5).sign == .plus)    // true
print(Decimal(-5).sign == .minus)  // true
print(Decimal(0).sign == .plus)    // true

// --- Magnitude ---
print(Decimal(-5).magnitude)   // 5
print(Decimal(5).magnitude)    // 5

// --- Comparison ---
print(Decimal(1) < Decimal(2))   // true
print(Decimal(1) == Decimal(1))  // true
print(Decimal(-1) < Decimal(0))  // true

// --- Multiply by zero ---
print(Decimal(123456) * Decimal(0))  // 0

// --- NaN propagation ---
print((Decimal.nan + Decimal(1)).isNaN)  // true

// --- 38-digit boundary arithmetic ---
print(Decimal(string: "9999999999999999999999999999999999999")! + Decimal(1))

// --- Division exact ---
print(Decimal(10) / Decimal(2))   // 5
print(Decimal(1) / Decimal(4))    // 0.25
