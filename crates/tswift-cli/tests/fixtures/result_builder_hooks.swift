// Result-builder block hooks (#126): buildPartialBlock left-fold,
// buildFinalResult wrapping the outermost value, and empty buildBlock().

// A builder declaring only the buildPartialBlock pair folds left-to-right.
@resultBuilder
struct PartialBuilder {
    static func buildExpression(_ value: String) -> String { "[" + value + "]" }
    static func buildPartialBlock(first: String) -> String { first }
    static func buildPartialBlock(accumulated: String, next: String) -> String {
        accumulated + ">" + next
    }
}

@PartialBuilder
func chain() -> String {
    "a"
    "b"
    "c"
}

// A builder declaring both buildBlock and the partial pair prefers partial-block,
// and wraps the outermost value once in buildFinalResult.
@resultBuilder
struct FinalBuilder {
    static func buildExpression(_ value: String) -> String { value }
    static func buildBlock(_ parts: String...) -> String { "block(" + parts.joined(separator: " ") + ")" }
    static func buildPartialBlock(first: String) -> String { first }
    static func buildPartialBlock(accumulated: String, next: String) -> String {
        accumulated + "+" + next
    }
    static func buildFinalResult(_ value: String) -> String { "final(" + value + ")" }
}

@FinalBuilder
func summary() -> String {
    "x"
    "y"
}

@FinalBuilder
func nothing() -> String {
}

print(chain())
print(summary())
print(nothing())
