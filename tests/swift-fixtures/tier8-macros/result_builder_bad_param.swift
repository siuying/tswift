// A result-builder attribute on a non-function-typed parameter is an error.
@resultBuilder
struct SB {
    static func buildBlock(_ parts: String...) -> String { parts.joined() }
}

func wrap(@SB _ content: Int) -> String { // expected-error{{function type}}
    "x"
}
