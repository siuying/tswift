// Result-builder control-flow semantics (#123): a sole `return` bypasses the
// builder; `guard` stays as control flow (not a component); a throwing body
// propagates the error through the synthesized builder calls.
@resultBuilder
struct StringBuilder {
    static func buildExpression(_ value: String) -> String { "[" + value + "]" }
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

print(guarded(5))
print(guarded(-1))
print(soleReturn())
print((try? risky(true)) ?? "nil")
do {
    _ = try risky(false)
} catch {
    print("caught")
}
