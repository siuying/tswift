// Int bit properties and Double classification members.
let n = 42
print(n.bitWidth)
print(n.nonzeroBitCount)
print(n.leadingZeroBitCount)
print(n.trailingZeroBitCount)
print((1).byteSwapped == 72057594037927936)

let z = 0
print(z.nonzeroBitCount, z.leadingZeroBitCount, z.trailingZeroBitCount)

var d = 3.5
d.negate()
print(d)
print((0.0).isZero, (1.0).isZero)
print((1.0).isNormal, (0.0).isNormal)
print((1.0).isSubnormal)
