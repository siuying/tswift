// A bare `if` in a builder body requires the builder to declare buildOptional;
// an `if/else` requires buildEither. Missing hooks are diagnosed (there is no
// runtime fallback).
@resultBuilder
struct SB {
    static func buildExpression(_ v: String) -> String { v }
    static func buildBlock(_ parts: String...) -> String { parts.joined(separator: " ") }
    static func buildEither(first: String) -> String { first }
    static func buildEither(second: String) -> String { second }
}

@SB
func g(_ b: Bool) -> String {
    if b { // expected-error{{requires the builder to declare 'buildOptional'}}
        "x"
    }
}
