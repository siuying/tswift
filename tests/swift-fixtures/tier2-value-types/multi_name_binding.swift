// expected-no-diagnostics
// Tier 2a — multi-name bindings: comma-separated stored properties sharing one
// type annotation, plus comma-separated local bindings with per-name
// initializers.

struct Vec4 {
    var a, b, c, d: Double
}

func sum(_ v: Vec4) -> Double {
    v.a + v.b + v.c + v.d
}

var x = 1, y = 2, z = 3
print(x + y + z)

var red, green, blue: Double
red = 0.1
green = 0.2
blue = 0.3
print(red, green, blue)

let total = sum(Vec4(a: 1.0, b: 2.0, c: 3.0, d: 4.0))
print(total)
