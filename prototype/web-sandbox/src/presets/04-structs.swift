// Struct: value semantics, mutating, computed props
struct Point {
    var x: Double
    var y: Double

    var magnitude: Double { (x*x + y*y).squareRoot() }

    mutating func translate(dx: Double, dy: Double) {
        x += dx; y += dy
    }

    func scaled(by factor: Double) -> Point {
        Point(x: x * factor, y: y * factor)
    }
}

var p = Point(x: 3, y: 4)
print("p = (\(p.x), \(p.y)), |p| = \(p.magnitude)")

var q = p           // value copy
q.translate(dx: 10, dy: 0)
print("after translating q: p=\(p.x), q=\(q.x)  ← independent copies")

let big = p.scaled(by: 2)
print("scaled: (\(big.x), \(big.y))")

// Struct with nested type
struct Matrix2x2 {
    var a, b, c, d: Double
    func determinant() -> Double { a*d - b*c }
}
let m = Matrix2x2(a: 1, b: 2, c: 3, d: 4)
print("det = \(m.determinant())")
