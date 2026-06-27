// expected-no-diagnostics
// Tier 8 — result-builder type-based overload selection (#124): buildExpression
// overloads separable by distinct scalar types are resolvable (the interpreter
// dispatches by the component's runtime type); no ambiguity diagnostic.

@resultBuilder
struct StringBuilder {
    static func buildExpression(_ value: String) -> String { value }
    static func buildExpression(_ value: Int) -> String { "\(value)" }
    static func buildBlock(_ parts: String...) -> String { parts.joined(separator: " ") }
}

@StringBuilder
func mixed() -> String {
    "hello"
    42
}
