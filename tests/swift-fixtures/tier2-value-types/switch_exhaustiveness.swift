// Tier 2 — switch exhaustiveness diagnostics. A switch over an enum that omits
// cases and has no `default` (or `@unknown default`) is a compile-time error,
// just as in Swift.

enum Direction { case north, south, east, west }

func partial(_ d: Direction) -> String {
    switch d { // expected-error{{switch must be exhaustive}}
    case .north: return "n"
    case .south: return "s"
    }
}
