// expected-no-diagnostics
// Tier 8 — result-builder availability (#129): an `if #available(…)` branch
// wraps its component in buildLimitedAvailability before the surrounding
// buildOptional / buildEither handling.

@resultBuilder
struct StringBuilder {
    static func buildExpression(_ value: String) -> String { value }
    static func buildBlock(_ parts: String...) -> String { parts.joined(separator: " ") }
    static func buildEither(first: String) -> String { first }
    static func buildEither(second: String) -> String { second }
    static func buildOptional(_ part: String?) -> String { part ?? "" }
    static func buildLimitedAvailability(_ value: String) -> String { value }
}

@StringBuilder
func bare() -> String {
    if #available(iOS 13, *) {
        "modern"
    }
}

@StringBuilder
func withElse() -> String {
    if #available(iOS 13, *) {
        "modern"
    } else {
        "legacy"
    }
}
