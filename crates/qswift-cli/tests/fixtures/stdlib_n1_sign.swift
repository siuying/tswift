// N1 follow-up — Double.sign and the builtin FloatingPointSign enum.

print((-3.5).sign == .minus, (3.5).sign == .minus)
print((-0.0).sign == .minus, (0.0).sign == .plus)
print(Double.infinity.sign, (-2.0).sign)

let s = (-1.0).sign
switch s {
case .minus: print("neg")
case .plus: print("pos")
}

print(FloatingPointSign.minus.rawValue, FloatingPointSign.plus.rawValue)
