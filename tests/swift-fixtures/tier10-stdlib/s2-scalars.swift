// expected-no-diagnostics
// Tier 10a/S2 — core scalar values.

let s = 17.signum()
let mult = 12.isMultiple(of: 4)
let qr = 17.quotientAndRemainder(dividingBy: 5)
let mag = (-9).magnitude

let root = 9.0.squareRoot()
let r = 2.6.rounded()
let trunc = 7.5.truncatingRemainder(dividingBy: 2.0)
let dm = (-3.5).magnitude
let nan = (0.0 / 0.0).isNaN
let fin = 3.0.isFinite

let parsedInt = Int("42")
let parsedDouble = Double("3.14")
let parsedBool = Bool("true")
let widened = UInt8(255)
let truncated = Int(3.9)

let wrapped = 2_000_000_000 &+ 2_000_000_000

let _ = (s, mult, qr, mag, root, r, trunc, dm, nan, fin,
         parsedInt, parsedDouble, parsedBool, widened, truncated, wrapped)
