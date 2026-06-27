// Walking-skeleton result builder: an expression-only `@Builder func` body is
// folded at sema time into `Builder.buildBlock(Builder.buildExpression(...))`
// calls. Declarations in the body stay in place and feed later components.
@resultBuilder
struct StringBuilder {
    static func buildExpression(_ value: String) -> String {
        "[" + value + "]"
    }

    static func buildBlock(_ parts: String...) -> String {
        parts.joined(separator: " ")
    }
}

@StringBuilder
func greeting() -> String {
    let prefix = "Hello"
    prefix
    "World"
}

@StringBuilder
func empty() -> String {
}

print(greeting())
print("[" + empty() + "]")
