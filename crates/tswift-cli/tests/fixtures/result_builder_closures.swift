// Result-builder contextual closures (#127): a closure literal passed to a
// @Builder parameter is transformed as a builder body — including trailing-
// closure syntax, @escaping parameters, and generic builder parameters. A
// closure passed by name (not a literal) is still transformed by the runtime.
@resultBuilder
struct StringBuilder {
    static func buildExpression(_ value: String) -> String { "[" + value + "]" }
    static func buildBlock(_ parts: String...) -> String { parts.joined(separator: " ") }
}

func wrap(@StringBuilder _ content: () -> String) -> String {
    "wrapped(" + content() + ")"
}

func wrapEscaping(@StringBuilder _ content: @escaping () -> String) -> String {
    "esc(" + content() + ")"
}

func makeGeneric<C>(@StringBuilder _ content: () -> C) -> C {
    content()
}

// Trailing-closure DSL.
print(wrap {
    "one"
    "two"
})

// @escaping @Builder parameter.
print(wrapEscaping {
    "a"
    "b"
})

// Generic builder parameter.
print(makeGeneric {
    "g1"
    "g2"
})

// A closure formed without builder context is an ordinary multi-statement
// closure (it returns its last value). Passing it by name does NOT re-apply the
// parameter's result builder — matching swiftc, since the result-builder
// transform only rewrites closure *literals* at the call site. So `content()`
// here yields "last", and `wrap(plain)` prints `wrapped(last)`.
let plain = {
    "plain"
    "last"
}
print(wrap(plain))
print(plain())
