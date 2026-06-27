// expected-no-diagnostics
// Tier 8 — result-builder declaration targets (#121): the transform applies to
// computed-property getters, subscript getters, explicit get accessors, and
// nested @resultBuilder declarations.

@resultBuilder
struct StringBuilder {
    static func buildExpression(_ value: String) -> String { value }
    static func buildBlock(_ parts: String...) -> String { parts.joined(separator: " ") }
}

struct View {
    let name: String

    @StringBuilder
    var body: String {
        "View"
        name
    }

    @StringBuilder
    subscript(_ tag: String) -> String {
        get {
            "tag"
            tag
        }
    }
}

struct Outer {
    @resultBuilder
    struct Inner {
        static func buildExpression(_ value: String) -> String { value }
        static func buildBlock(_ parts: String...) -> String { parts.joined() }
    }

    @Inner
    func make(_ who: String) -> String {
        "Hi"
        who
    }
}
