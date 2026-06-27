// buildEither must take a single first:/second: parameter.
@resultBuilder
struct SB {
    static func buildBlock(_ parts: String...) -> String { parts.joined() }
    static func buildEither(_ value: String) -> String { value } // expected-error{{'first:' or 'second:'}}
}
