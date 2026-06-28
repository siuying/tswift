// Arithmetic, comparison and bitwise operators across the scalar types.
// Int arithmetic and comparisons.
print(7 - 2, 7 * 2, 7 / 2, 7 % 2)
print(7 == 7, 7 != 2, 7 < 2, 7 <= 7, 7 > 2, 7 >= 7)
// Int bitwise and masking shifts.
print(6 & 3, 6 | 3, 6 ^ 3, 1 &<< 4, 64 &>> 2)
// Int compound assignment.
var i = 100
i -= 1
i *= 2
i /= 3
i %= 40
print(i)
var bits = 12
bits &= 10
bits |= 1
bits ^= 3
print(bits)
// Double arithmetic and compound assignment.
print(3.0 - 1.0, 3.0 * 2.0, 3.0 / 2.0)
var d = 5.0
d -= 1.0
d *= 2.0
d /= 4.0
print(d)
// String ordering.
print("a" < "b")
// Bool logic.
print(true && false, true || false, true == true)
