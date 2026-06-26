// expected-no-diagnostics
// Tier 2 — the exhaustiveness checker must not flag a switch that covers every
// enum case, whether via all cases, a `default`, an `@unknown default`, a
// catch-all binding, or an irrefutable associated-value pattern.

enum Light { case red, yellow, green }

func allCases(_ l: Light) -> Int {
    switch l {
    case .red: return 0
    case .yellow, .green: return 1
    }
}

func withDefault(_ l: Light) -> Int {
    switch l {
    case .red: return 0
    default: return 1
    }
}

func withUnknownDefault(_ l: Light) -> Int {
    switch l {
    case .red: return 0
    @unknown default: return 1
    }
}

func withCatchAll(_ l: Light) -> Int {
    switch l {
    case .red: return 0
    case let other: _ = other; return 1
    }
}

enum Payload { case value(Int), none }

func withIrrefutablePayload(_ p: Payload) -> Int {
    switch p {
    case .value(let n): return n
    case .none: return 0
    }
}

let _ = (
    allCases(.red),
    withDefault(.green),
    withUnknownDefault(.yellow),
    withCatchAll(.red),
    withIrrefutablePayload(.none)
)
