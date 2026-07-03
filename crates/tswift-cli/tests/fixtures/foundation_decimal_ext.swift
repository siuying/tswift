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
