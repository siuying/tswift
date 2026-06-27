// frontend-gap: Tier C result-builder method selection — choosing a
// buildExpression overload by protocol conformance / bidirectional contextual
// type — needs a constraint solver the forward-only sema does not have. Documented
// as a known limitation (plan docs/plan/result-builders.md §4.1), not miscompiled.
protocol Drawable {}
struct Circle: Drawable {}

@resultBuilder
struct DrawBuilder {
    static func buildExpression<T: Drawable>(_ value: T) -> String { "" }
    static func buildExpression(_ value: String) -> String { value }
    static func buildBlock(_ parts: String...) -> String { parts.joined() }
}

@DrawBuilder
func scene() -> String {
    Circle()
    "label"
}
