// A build method overloaded only by unmodelled (user) types cannot be resolved
// by the forward-only type checker, so it is diagnosed rather than guessed.
struct Foo {}
struct Bar {}

@resultBuilder
struct B { // expected-error{{ambiguous result-builder method}}
    static func buildExpression(_ value: Foo) -> String { "" }
    static func buildExpression(_ value: Bar) -> String { "" }
    static func buildBlock(_ parts: String...) -> String { parts.joined() }
}
