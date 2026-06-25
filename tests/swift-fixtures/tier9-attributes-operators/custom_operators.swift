// expected-no-diagnostics
// Tier 9b — prefix/infix operator declarations, precedencegroup, overloading.

precedencegroup ExponentPrecedence {
    higherThan: MultiplicationPrecedence
    associativity: right
}

infix operator ** : ExponentPrecedence

func ** (base: Double, exponent: Double) -> Double {
    var result = 1.0
    var n = Int(exponent)
    while n > 0 {
        result *= base
        n -= 1
    }
    return result
}

struct Vector2D: Equatable {
    var x: Double
    var y: Double

    static func + (lhs: Vector2D, rhs: Vector2D) -> Vector2D {
        Vector2D(x: lhs.x + rhs.x, y: lhs.y + rhs.y)
    }

    static prefix func - (operand: Vector2D) -> Vector2D {
        Vector2D(x: -operand.x, y: -operand.y)
    }
}

let power = 2.0 ** 10.0
let sum = Vector2D(x: 1, y: 2) + Vector2D(x: 3, y: 4)
let negated = -sum

let _ = (power, sum, negated)
