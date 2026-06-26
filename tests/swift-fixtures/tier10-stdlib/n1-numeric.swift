// expected-no-diagnostics
// Tier 10a/N1 — numeric completion: Double constants/instance + Int bit/overflow.

let pi = Double.pi
let inf = Double.infinity
let nan = Double.nan
let great = Double.greatestFiniteMagnitude
let leastNonzero = Double.leastNonzeroMagnitude

var rounding = 2.6
rounding.round()
var negating = 3.5
negating.negate()

let zero = 0.0.isZero
let up = 1.0.nextUp
let exp = 8.0.exponent
let sig = 0.75.significand

let width = (0).bitWidth
let lead = 255.leadingZeroBitCount
let trail = 8.trailingZeroBitCount
let nonzero = 7.nonzeroBitCount
let swapped = 1.byteSwapped

let added = Int8(100).addingReportingOverflow(50)
let subtracted = 5.subtractingReportingOverflow(3)
let multiplied = 10.multipliedReportingOverflow(by: 3)

let _ = (pi, inf, nan, great, leastNonzero, rounding, negating, zero, up, exp,
         sig, width, lead, trail, nonzero, swapped, added, subtracted, multiplied)
