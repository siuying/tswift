// N1 — numeric completion: Double constants/instance + Int bit/overflow ops.

// Double static type constants.
print(Double.pi)
print(Double.infinity, -Double.infinity)
print(Double.nan)
// Extreme-magnitude constants verified by comparison (formatting is exponential
// in Swift; the runtime renders the full decimal, so compare instead of print).
print(Double.greatestFiniteMagnitude > 1e308, Double.leastNonzeroMagnitude > 0.0)

// Double mutating methods.
var r = 2.6
r.round()
print(r)
var n = 3.5
n.negate()
print(n)

// Double instance properties.
print(0.0.isZero, 1.0.isZero)
print(1.0.nextUp)
print(8.0.exponent, 0.5.exponent, 1.0.exponent)
print(8.0.significand, 0.75.significand)

// Int bit-representation properties (width-aware).
print((0).bitWidth, Int8(0).bitWidth)
print(255.leadingZeroBitCount, 1.leadingZeroBitCount)
print(8.trailingZeroBitCount, 1.trailingZeroBitCount)
print(7.nonzeroBitCount)
print(1.byteSwapped)

// Int reporting-overflow methods (partialValue wraps; overflow flags it).
let a = Int8(100).addingReportingOverflow(50)
print(a.0, a.1)
let m = 10.multipliedReportingOverflow(by: 3)
print(m.0, m.1)
let s = 5.subtractingReportingOverflow(3)
print(s.0, s.1)
