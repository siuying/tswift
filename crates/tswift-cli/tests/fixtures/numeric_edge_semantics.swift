// Integer semantics
print(Int.max &+ 1 == Int.min)
print(7 % 3, -7 % 3, 7 % -3)
let a: Int8 = 100
print(a &+ a)
print(1 << 62)
print(Int8(-128) &- 1)
// Float semantics
let nan = Double.nan
print(nan == nan)
print(0.1 + 0.2)
print(1.0 / 0.0)
print(-0.0 == 0.0)
print(3.0.rounded(), (-3.5).rounded(), 2.5.rounded())
// Integer division truncates toward zero
print(7 / 2, -7 / 2)
