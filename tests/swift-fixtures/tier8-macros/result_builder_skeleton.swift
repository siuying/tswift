// expected-no-diagnostics
// Tier 8 — result-builder walking skeleton: an expression-only `@Builder func`
// body is rewritten by the sema transform into `Builder.buildBlock(
// Builder.buildExpression(...))` calls. The synthesized `$build`-prefixed
// bindings and the now-ordinary static calls must annotate without diagnostics.

@resultBuilder
struct StringBuilder {
    static func buildExpression(_ value: String) -> String { value }
    static func buildBlock(_ parts: String...) -> String { parts.joined(separator: " ") }
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
