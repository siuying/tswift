import Foundation

// `+` — concatenation of two IndexPaths.
var a = IndexPath(indexes: [1, 2])
let b = IndexPath(indexes: [3, 4])
let c = a + b
print(c)

// `+=` — in-place concatenation.
a += b
print(a)

// Concatenating with an empty IndexPath is a no-op on the other operand.
let empty = IndexPath()
print(empty + a)
print(a + empty)

var d = IndexPath(index: 7)
d += IndexPath(indexes: [8, 9])
print(d)

// `subscript(position:)` — read each element by position.
let ip = IndexPath(indexes: [10, 20, 30])
print(ip[0])
print(ip[1])
print(ip[2])
