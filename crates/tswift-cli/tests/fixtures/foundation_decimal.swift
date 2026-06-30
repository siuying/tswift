import Foundation

// Exact base-10 arithmetic: 0.1 + 0.2 == 0.3 (Double gets this wrong).
let a = Decimal(string: "0.1")!
let b = Decimal(string: "0.2")!
print((a + b) == Decimal(string: "0.3")!)
print((a + b).description)

// Construction.
print(Decimal().description)
print(Decimal(42).description)
print(Decimal(1.25).description)
print(Decimal(string: "not a number") == nil)

// Arithmetic and mixed-literal operands.
print((Decimal(2) * Decimal(string: "1.5")!).description)
print((Decimal(1) / Decimal(8)).description)
print((Decimal(string: "3.25")! + 1).description)
print((-Decimal(string: "12.50")!).description)

// Compound assignment.
var running = Decimal(string: "1.5")!
running += Decimal(1)
running *= Decimal(2)
print(running.description)

// Comparison.
print(Decimal(10) < Decimal(string: "10.5")!)
print(Decimal(string: "0.30")! == Decimal(string: "0.3")!)

// Properties.
print(Decimal(0).isZero)
print(Decimal(string: "-7.5")!.magnitude.description)

// Rounding modes.
let half = Decimal(string: "2.5")!
print(half.rounded(0, .plain).description)
print(half.rounded(0, .down).description)
print(half.rounded(0, .up).description)
print(half.rounded(0, .bankers).description)
print(Decimal(string: "3.5")!.rounded(0, .bankers).description)
print(Decimal(string: "1.2345")!.rounded(2, .plain).description)
