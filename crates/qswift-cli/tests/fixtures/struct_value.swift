struct Point { var x: Int; var y: Int
    mutating func move(dx: Int) { x += dx }
    var magnitude: Int { x*x + y*y }
}
var a = Point(x: 1, y: 2)
var b = a
b.move(dx: 10)
print(a.x, b.x)
print(b.magnitude)
print(a)
