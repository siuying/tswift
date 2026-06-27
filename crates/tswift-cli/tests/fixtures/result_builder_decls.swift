// Result-builder declaration targets (#121): the transform applies to computed
// property getters, subscript getters, explicit get accessors, and nested
// @resultBuilder declarations — not just @Builder func.
@resultBuilder
struct StringBuilder {
    static func buildExpression(_ value: String) -> String { "[" + value + "]" }
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

// A nested @resultBuilder, recognized because Symbols walks nested scopes.
struct Outer {
    @resultBuilder
    struct Inner {
        static func buildExpression(_ value: String) -> String { "<" + value + ">" }
        static func buildBlock(_ parts: String...) -> String { parts.joined(separator: "|") }
    }

    @Inner
    func make(_ who: String) -> String {
        "Hi"
        who
    }
}

let v = View(name: "Home")
print(v.body)
print(v["main"])
print(Outer().make("Swift"))
