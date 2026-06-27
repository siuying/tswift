// Integer literals coerce to Double in a floating context: annotated bindings,
// struct/enum fields, function parameters, and mixed arithmetic.
let r: Double = 5
print(r)

struct Point { var x: Double; var y: Double
    var magnitude: Double { (x * x + y * y).squareRoot() }
}
let p = Point(x: 3, y: 4)
print(p.x, p.y, p.magnitude)

enum Shape {
    case circle(radius: Double)
    case rect(width: Double, height: Double)
    var area: Double {
        switch self {
        case .circle(let r): return 3.14159 * r * r
        case .rect(let w, let h): return w * h
        }
    }
}
print(Shape.rect(width: 4, height: 6).area)

func scale(_ v: Double) -> Double { v * 2 }
print(scale(5))

var d = 0.0
d += 1
print(d)

// Integer arithmetic is unaffected.
print(7 / 2, 7 % 2)
