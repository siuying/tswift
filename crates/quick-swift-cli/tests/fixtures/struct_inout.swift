struct Vec { var dx: Int; var dy: Int }
func bump(_ n: inout Int) { n += 1 }
func scale(_ v: inout Vec, by k: Int) { v.dx *= k; v.dy *= k }
var n = 41
bump(&n)
print(n)
var v = Vec(dx: 2, dy: 3)
scale(&v, by: 10)
print(v.dx, v.dy)
bump(&v.dx)
print(v.dx)
