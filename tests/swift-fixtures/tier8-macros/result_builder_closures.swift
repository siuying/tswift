// expected-no-diagnostics
// Tier 8 — result-builder contextual closures (#127): a closure literal passed
// to a @Builder parameter is transformed as a builder body, including trailing-
// closure syntax, @escaping parameters, and generic builder parameters.

@resultBuilder
struct StringBuilder {
    static func buildExpression(_ value: String) -> String { value }
    static func buildBlock(_ parts: String...) -> String { parts.joined(separator: " ") }
}

func wrap(@StringBuilder _ content: () -> String) -> String {
    content()
}

func wrapEscaping(@StringBuilder _ content: @escaping () -> String) -> String {
    content()
}

func makeGeneric<C>(@StringBuilder _ content: () -> C) -> C {
    content()
}

let a = wrap {
    "one"
    "two"
}

let b = wrapEscaping {
    "a"
    "b"
}

let c = makeGeneric {
    "g1"
    "g2"
}
