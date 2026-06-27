// expected-no-diagnostics
// Tier 8 — result-builder control-flow semantics (#123): a sole `return`
// bypasses the builder; `guard` stays as control flow (not a component); a
// throwing body propagates errors through the synthesized calls. (async builder
// closures are gated on async-closure support — tracked separately.)

@resultBuilder
struct StringBuilder {
    static func buildExpression(_ value: String) -> String { value }
    static func buildBlock(_ parts: String...) -> String { parts.joined(separator: " ") }
}

enum MyError: Error { case bad }

@StringBuilder
func guarded(_ x: Int) -> String {
    guard x > 0 else { return "neg" }
    "pos"
    "\(x)"
}

@StringBuilder
func soleReturn() -> String {
    return "direct"
}

@StringBuilder
func risky(_ ok: Bool) throws -> String {
    guard ok else { throw MyError.bad }
    "fine"
}
