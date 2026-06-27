// Result-builder for-loops (#125): a `for` folds into a fresh accumulator that
// the real loop appends to, then `buildArray(accumulator)`. Pattern bindings,
// `where`, `break`/`continue`, and labeled loops are preserved.
@resultBuilder
struct StringBuilder {
    static func buildExpression(_ value: String) -> String { "[" + value + "]" }
    static func buildBlock(_ parts: String...) -> String { parts.joined(separator: " ") }
    static func buildArray(_ parts: [String]) -> String { parts.joined(separator: ",") }
    static func buildEither(first: String) -> String { first }
    static func buildEither(second: String) -> String { second }
    static func buildOptional(_ part: String?) -> String { part ?? "_" }
}

@StringBuilder
func words() -> String {
    "head"
    for word in ["Swift", "Rust"] {
        word
    }
}

@StringBuilder
func evens() -> String {
    for n in [1, 2, 3, 4] where n % 2 == 0 {
        "\(n)"
    }
}

@StringBuilder
func pairs() -> String {
    for (k, v) in [(1, "a"), (2, "b")] {
        "\(k)\(v)"
    }
}

@StringBuilder
func nested() -> String {
    for n in [1, 2, 3] {
        if n == 2 {
            "two"
        } else {
            "\(n)"
        }
    }
}

print(words())
print(evens())
print(pairs())
print(nested())
