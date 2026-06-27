@resultBuilder
struct StringBuilder {
    static func buildExpression(_ value: String) -> String { value }
    static func buildBlock(_ parts: String...) -> String { parts.joined(separator: " ") }
}

@StringBuilder
func greeting() -> String {
    let prefix = "Hi"
    prefix
    "there"
}
