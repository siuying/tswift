// expected-no-diagnostics
// Tier 2a — enums: simple/associated/raw values, indirect, methods, CaseIterable.

enum Direction: CaseIterable {
    case north, south, east, west

    var opposite: Direction {
        switch self {
        case .north: return .south
        case .south: return .north
        case .east: return .west
        case .west: return .east
        }
    }
}

enum Barcode {
    case upc(Int, Int, Int, Int)
    case qr(String)
}

enum Planet: Int {
    case mercury = 1, venus, earth
}

indirect enum Expr {
    case literal(Int)
    case add(Expr, Expr)
}

func evaluate(_ e: Expr) -> Int {
    switch e {
    case .literal(let n): return n
    case .add(let a, let b): return evaluate(a) + evaluate(b)
    }
}

// Floating associated values accept integer literals at construction.
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

let code = Barcode.qr("ABC-123")
let here = Planet.earth
let everyDirection = Direction.allCases
let folded = evaluate(.add(.literal(1), .add(.literal(2), .literal(3))))
let rectArea = Shape.rect(width: 4, height: 6).area

let _ = (Direction.north.opposite, code, here.rawValue, everyDirection.count, folded, rectArea)
