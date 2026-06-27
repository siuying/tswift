// expected-no-diagnostics
// Tier 8 — result-builder declarations and DSL body transforms.

@resultBuilder
struct StringBuilder {
    static func buildExpression(_ value: String) -> String { value }
    static func buildBlock(_ parts: String...) -> String { parts.joined(separator: " ") }
    static func buildEither(first: String) -> String { first }
    static func buildEither(second: String) -> String { second }
    static func buildOptional(_ part: String?) -> String { part ?? "" }
    static func buildArray(_ parts: [String]) -> String { parts.joined(separator: ",") }
}

@StringBuilder
func greeting(_ excited: Bool) -> String {
    "Hello"
    if excited {
        "World!"
    } else {
        "World"
    }
    for word in ["Swift", "Rust"] {
        word
    }
}

func wrap(@StringBuilder _ content: () -> String) -> String {
    content()
}

let _ = (greeting(true), wrap { "one"; "two" })
