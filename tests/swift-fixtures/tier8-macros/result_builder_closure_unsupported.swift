// The contextual-closure path is validated too: an unsupported construct in a
// closure literal passed to a @Builder parameter is diagnosed, not run silently.
@resultBuilder
struct SB {
    static func buildExpression(_ v: String) -> String { v }
    static func buildBlock(_ parts: String...) -> String { parts.joined(separator: " ") }
}

func wrap(@SB _ content: () -> String) -> String { content() }

let r = wrap {
    "a"
    while false { // expected-error{{'while'/'repeat'}}
        "x"
    }
    "b"
}
