// An explicit return mixed with components is an error.
@resultBuilder
struct SB {
    static func buildBlock(_ parts: String...) -> String { parts.joined() }
}

@SB
func g() -> String {
    "a"
    return "b" // expected-error{{explicit 'return'}}
}
