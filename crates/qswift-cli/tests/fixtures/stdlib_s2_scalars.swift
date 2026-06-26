// S2 — core scalar values.
print(17.signum(), (-9).signum(), 0.signum())
print(12.isMultiple(of: 4), 13.isMultiple(of: 4))
let qr = 17.quotientAndRemainder(dividingBy: 5)
print(qr.0, qr.1)
print((-9).magnitude)

print(9.0.squareRoot())
print(2.6.rounded(), (-2.6).rounded())
print(7.5.truncatingRemainder(dividingBy: 2.0))
print(2.5.magnitude, (-3.5).magnitude)

let nan = 0.0 / 0.0
print(nan.isNaN, 1.0.isNaN, (1.0 / 0.0).isInfinite, 3.0.isFinite)

print(Int("42") ?? -1, Int("oops") ?? -1)
print(Double("3.14") ?? -1.0, Double("x") ?? -1.0)
print(Bool("true") ?? false, Bool("nope") ?? false)

print(UInt8(255), Int16(-100), Int32(7))
print(Int(3.9), Double(5))

let big = 2_000_000_000
print(big &+ big)
print(big &* 2)
