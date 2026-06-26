// Enums: simple, associated values, raw values, indirect, CaseIterable
enum Direction: CaseIterable {
    case north, south, east, west
    var opposite: Direction {
        switch self {
        case .north: return .south; case .south: return .north
        case .east:  return .west;  case .west:  return .east
        }
    }
}
print("All directions: \(Direction.allCases)")
print("Opposite of north: \(Direction.north.opposite)")

enum Shape {
    case circle(radius: Double)
    case rectangle(width: Double, height: Double)

    var area: Double {
        switch self {
        case .circle(let r):             return 3.14159 * r * r
        case .rectangle(let w, let h):  return w * h
        }
    }
}
let shapes: [Shape] = [.circle(radius: 5), .rectangle(width: 4, height: 6)]
for s in shapes { print("area = \(s.area)") }

// Raw-value enum
enum Planet: Int { case mercury = 1, venus, earth, mars }
print("Earth = \(Planet.earth.rawValue)")

// Indirect (recursive) enum
indirect enum Expr {
    case num(Int); case add(Expr, Expr); case mul(Expr, Expr)
}
func eval(_ e: Expr) -> Int {
    switch e {
    case .num(let n): return n
    case .add(let a, let b): return eval(a) + eval(b)
    case .mul(let a, let b): return eval(a) * eval(b)
    }
}
// (2 + 3) * 4
print("(2+3)*4 = \(eval(.mul(.add(.num(2), .num(3)), .num(4))))")
