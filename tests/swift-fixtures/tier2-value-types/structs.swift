// expected-no-diagnostics
// Tier 2a — struct stored properties, memberwise init, methods, mutating,
// nested types, and value semantics.

struct Point {
    var x: Int
    var y: Int

    func distanceSquared() -> Int { x * x + y * y }

    mutating func translate(dx: Int, dy: Int) {
        x += dx
        y += dy
    }

    struct Pair {
        var a: Int
        var b: Int
    }
}

var p = Point(x: 3, y: 4)
let d = p.distanceSquared()
p.translate(dx: 1, dy: -1)
let nested = Point.Pair(a: 1, b: 2)

// Value semantics: assignment copies, so mutating `q` leaves `p` untouched.
var q = p
q.x = 100

// Integer literals coerce to Double in a floating field/binding context.
struct Vec2 {
    var x: Double
    var y: Double
    var length: Double { (x * x + y * y).squareRoot() }
}
let v = Vec2(x: 3, y: 4)
let radius: Double = 5

let _ = (d, p.x, q.x, nested.a, nested.b, v.length, radius)
