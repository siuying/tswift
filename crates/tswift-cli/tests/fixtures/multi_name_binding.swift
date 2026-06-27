// Multi-name binding: comma-separated names sharing one type annotation, and
// comma-separated initializers.
struct Vec4 { var a, b, c, d: Double }
let v = Vec4(a: 1.0, b: 2.0, c: 3.0, d: 4.0)
print(v.a, v.b, v.c, v.d)

var x = 1, y = 2, z = 3
print(x + y + z)

var red, green, blue: Double
red = 0.1
green = 0.2
blue = 0.3
print(red, green, blue)
