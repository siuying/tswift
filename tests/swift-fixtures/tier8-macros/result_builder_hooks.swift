// expected-no-diagnostics
// Tier 8 — result-builder block hooks (#126): buildPartialBlock left-fold,
// buildFinalResult wrapping the outermost value, and empty buildBlock().

@resultBuilder
struct PartialBuilder {
    static func buildExpression(_ value: String) -> String { value }
    static func buildPartialBlock(first: String) -> String { first }
    static func buildPartialBlock(accumulated: String, next: String) -> String {
        accumulated + next
    }
}

@PartialBuilder
func chain() -> String {
    "a"
    "b"
    "c"
}

@resultBuilder
struct FinalBuilder {
    static func buildExpression(_ value: String) -> String { value }
    static func buildBlock(_ parts: String...) -> String { parts.joined() }
    static func buildPartialBlock(first: String) -> String { first }
    static func buildPartialBlock(accumulated: String, next: String) -> String {
        accumulated + next
    }
    static func buildFinalResult(_ value: String) -> String { value }
}

@FinalBuilder
func summary() -> String {
    "x"
    "y"
}

@FinalBuilder
func nothing() -> String {
}
