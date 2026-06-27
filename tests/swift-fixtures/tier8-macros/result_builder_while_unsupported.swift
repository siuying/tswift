// while/repeat loops are unsupported in a result-builder body.
@resultBuilder
struct SB {
    static func buildBlock(_ parts: String...) -> String { parts.joined() }
}

@SB
func loopy() -> String {
    while true { // expected-error{{'while'/'repeat'}}
        "x"
    }
}
