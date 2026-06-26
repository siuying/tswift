// expected-no-diagnostics
// Tier 10/N1 follow-up — Double.sign and FloatingPointSign.

let neg = (-3.5).sign
let pos = (3.5).sign
let isNeg = neg == .minus
let raw = FloatingPointSign.minus.rawValue

var label = "pos"
switch (-1.0).sign {
case .minus: label = "neg"
case .plus: label = "pos"
}

let _ = (neg, pos, isNeg, raw, label)
