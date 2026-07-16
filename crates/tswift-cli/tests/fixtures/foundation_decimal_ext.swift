import Foundation

// ── Static constants ──────────────────────────────────────────────────────────
print(Decimal.radix)
print(Decimal.greatestFiniteMagnitude > 0)
print(Decimal.leastFiniteMagnitude < 0)
print(Decimal.leastNonzeroMagnitude > 0)
print(Decimal.leastNormalMagnitude == Decimal.leastNonzeroMagnitude)
// leastNonzeroMagnitude matches Foundation's real value (10^-127)
print(Decimal.leastNonzeroMagnitude.description)

// ── ulp — 38-significant-digit model ─────────────────────────────────────────
// ulp(x) = 10^(floor(log10(|x|)) - 38), floored at 10^-128
print(Decimal(1).ulp.description)               // 10^-38
print(Decimal(string: "1.5")!.ulp.description)  // 10^-38 (same magnitude class)
print(Decimal(string: "0.01")!.ulp.description) // 10^-40

// ── nextUp / nextDown ─────────────────────────────────────────────────────────
print(Decimal(string: "1.5")!.nextUp.description)   // 1.5 + 10^-38
print(Decimal(string: "1.5")!.nextDown.description) // 1.5 - 10^-38
print(Decimal(1).nextUp.description)                // 1 + 10^-38
print(Decimal(1).nextDown.description)              // 1 - 10^-38
print(Decimal.nan.nextUp.isNaN)
print(Decimal.nan.nextDown.isNaN)

// ── isTotallyOrdered ──────────────────────────────────────────────────────────
// Foundation: any NaN operand returns false
print(Decimal(3).isTotallyOrdered(belowOrEqualTo: Decimal(5)))     // true
print(Decimal(5).isTotallyOrdered(belowOrEqualTo: Decimal(3)))     // false
print(Decimal(3).isTotallyOrdered(belowOrEqualTo: Decimal(3)))     // true
print(Decimal.nan.isTotallyOrdered(belowOrEqualTo: Decimal(5)))    // false
print(Decimal(5).isTotallyOrdered(belowOrEqualTo: Decimal.nan))    // false
print(Decimal.nan.isTotallyOrdered(belowOrEqualTo: Decimal.nan))   // false

// ── floatingPointClass ────────────────────────────────────────────────────────
print(Decimal(5).floatingPointClass == .positiveNormal)    // true
print(Decimal(-5).floatingPointClass == .negativeNormal)   // true
print(Decimal(0).floatingPointClass == .positiveZero)      // true
print(Decimal.nan.floatingPointClass == .quietNaN)         // true

// ── formatted() ───────────────────────────────────────────────────────────────
print(Decimal(1234567).formatted())             // 1,234,567
print(Decimal(string: "1234.56")!.formatted())  // 1,234.56
print(Decimal(-1234).formatted())               // -1,234
print(Decimal(0).formatted())                   // 0
print(Decimal(999).formatted())                 // 999

// ── formTruncatingRemainder(dividingBy:) ─────────────────────────────────────
// self - other * (self / other).rounded(.towardZero)
var r0 = Decimal(10)
r0.formTruncatingRemainder(dividingBy: Decimal(3))
print(r0.description)                            // 1
var r1 = Decimal(string: "10.5")!
r1.formTruncatingRemainder(dividingBy: Decimal(string: "3.2")!)
print(r1.description)                            // 0.9
var r2 = Decimal(-10)
r2.formTruncatingRemainder(dividingBy: Decimal(3))
print(r2.description)                            // -1 (sign follows dividend)
var r3 = Decimal(7)
r3.formTruncatingRemainder(dividingBy: Decimal.nan)
print(r3.isNaN)                                  // true

// ── signalingNaN / isSignaling ───────────────────────────────────────────────
// Decimal has no distinct signaling NaN: value is NaN, predicate is false.
print(Decimal.signalingNaN.isNaN)               // true
print(Decimal.signalingNaN.isSignaling)         // false
print(Decimal(5).isSignaling)                   // false

// ── /= operator ──────────────────────────────────────────────────────────────
var q = Decimal(10)
q /= Decimal(4)
print(q.description)                            // 2.5
