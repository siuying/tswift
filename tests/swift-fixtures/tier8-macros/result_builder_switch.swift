// expected-no-diagnostics
// Tier 8 — result-builder switch (#128): a switch lowers to a balanced
// buildEither tree over its cases and stays a real switch (patterns, bindings,
// where guards, default preserved).

@resultBuilder
struct StringBuilder {
    static func buildExpression(_ value: String) -> String { value }
    static func buildBlock(_ parts: String...) -> String { parts.joined(separator: " ") }
    static func buildEither(first: String) -> String { first }
    static func buildEither(second: String) -> String { second }
}

@StringBuilder
func describe(_ n: Int) -> String {
    "n ="
    switch n {
    case 0:
        "zero"
    case let x where x < 0:
        "neg"
    default:
        "pos"
    }
}

@StringBuilder
func grade(_ score: Int) -> String {
    switch score {
    case 90...100:
        "A"
    case 80..<90:
        "B"
    case 70..<80:
        "C"
    default:
        "F"
    }
}
