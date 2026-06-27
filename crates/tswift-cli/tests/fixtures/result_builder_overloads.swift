// Result-builder type-based overload selection (#124): two buildExpression
// overloads separable by scalar type are dispatched by the component's runtime
// type after the transform erases the builder.
@resultBuilder
struct StringBuilder {
    static func buildExpression(_ value: String) -> String { "s(" + value + ")" }
    static func buildExpression(_ value: Int) -> String { "i(\(value))" }
    static func buildExpression(_ value: Bool) -> String { "b(\(value))" }
    static func buildBlock(_ parts: String...) -> String { parts.joined(separator: " ") }
}

@StringBuilder
func mixed() -> String {
    "hello"
    42
    true
}

print(mixed())
