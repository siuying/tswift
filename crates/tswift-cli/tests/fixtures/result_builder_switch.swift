// Result-builder switch (#128): a switch lowers to a balanced buildEither tree
// over its cases, staying a real switch (patterns, bindings, where guards, and
// default preserved). default lands on the final second.
@resultBuilder
struct StringBuilder {
    static func buildExpression(_ value: String) -> String { "[" + value + "]" }
    static func buildBlock(_ parts: String...) -> String { parts.joined(separator: " ") }
    static func buildEither(first: String) -> String { "F(" + first + ")" }
    static func buildEither(second: String) -> String { "S(" + second + ")" }
}

@StringBuilder
func describe(_ n: Int) -> String {
    "n ="
    switch n {
    case 0:
        "zero"
    case let x where x < 0:
        "neg"
    default:
        "pos"
    }
}

@StringBuilder
func grade(_ score: Int) -> String {
    switch score {
    case 90...100:
        "A"
    case 80..<90:
        "B"
    case 70..<80:
        "C"
    default:
        "F"
    }
}

print(describe(0))
print(describe(-5))
print(describe(7))
print(grade(95))
print(grade(85))
print(grade(75))
print(grade(40))
