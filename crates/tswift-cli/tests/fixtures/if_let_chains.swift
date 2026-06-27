// Multiple optional bindings and a boolean clause in one `if`/`while` condition.
let a: Int? = 1
let b: Int? = 2
if let a = a, let b = b, a < b {
    print("\(a) < \(b)")
}

let missing: Int? = nil
if let a = a, let m = missing {
    print("both: \(a) \(m)")
} else {
    print("one is nil")
}

// Shorthand binding plus a where-style boolean clause.
let score: Int? = 90
if let score, score >= 80 {
    print("pass \(score)")
}

// `while let` peeling optionals from a producer until it yields nil.
var counter = 3
func nextTicket() -> Int? {
    guard counter > 0 else { return nil }
    counter -= 1
    return counter + 1
}
while let ticket = nextTicket() {
    print("ticket \(ticket)")
}
