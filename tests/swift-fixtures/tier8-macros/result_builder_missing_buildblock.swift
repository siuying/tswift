// A @resultBuilder type without buildBlock (or the partial pair) is an error.
@resultBuilder
struct Bad { // expected-error{{must provide a static 'buildBlock'}}
    static func buildExpression(_ v: String) -> String { v }
}
