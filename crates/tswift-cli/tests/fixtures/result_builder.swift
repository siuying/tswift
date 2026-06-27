@resultBuilder
struct StringBuilder {
    static func buildExpression(_ value: String) -> String {
        "[" + value + "]"
    }

    static func buildBlock(_ parts: String...) -> String {
        parts.joined(separator: " ")
    }

    static func buildEither(first: String) -> String {
        "first=" + first
    }

    static func buildEither(second: String) -> String {
        "second=" + second
    }

    static func buildOptional(_ part: String?) -> String {
        part ?? "none"
    }

    static func buildArray(_ parts: [String]) -> String {
        parts.joined(separator: ",")
    }
}

@StringBuilder
func greeting(_ excited: Bool) -> String {
    let prefix = "Hello"
    prefix
    if excited {
        "World!"
    } else {
        "World"
    }
    if false {
        "unused"
    }
    for word in ["Swift", "Rust"] {
        word
    }
}

func wrap(@StringBuilder _ content: () -> String) -> String {
    "wrapped(" + content() + ")"
}

print(greeting(true))
print(greeting(false))
print(wrap {
    "one"
    "two"
})
let plain = {
    "plain"
    "last"
}
print(plain())
print(wrap(plain))
print(plain())
