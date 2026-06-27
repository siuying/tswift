// A build method must be declared static.
@resultBuilder
struct SB {
    static func buildBlock(_ parts: String...) -> String { parts.joined() }
    func buildExpression(_ v: String) -> String { v } // expected-error{{must be declared 'static'}}
}
