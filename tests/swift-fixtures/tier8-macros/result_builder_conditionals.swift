// expected-no-diagnostics
// Tier 8 — result-builder conditionals (#120): if/else lowers through
// buildEither, a bare if through buildOptional, if-let binds then transforms
// the branch, and else-if chains nest. The synthesized control flow and calls
// must annotate without diagnostics.

@resultBuilder
struct StringBuilder {
    static func buildExpression(_ value: String) -> String { value }
    static func buildBlock(_ parts: String...) -> String { parts.joined(separator: " ") }
    static func buildEither(first: String) -> String { first }
    static func buildEither(second: String) -> String { second }
    static func buildOptional(_ part: String?) -> String { part ?? "" }
}

@StringBuilder
func ifElse(_ flag: Bool) -> String {
    "head"
    if flag {
        "yes"
    } else {
        "no"
    }
}

@StringBuilder
func bareIf(_ flag: Bool) -> String {
    if flag {
        "present"
    }
}

@StringBuilder
func ifLet(_ name: String?) -> String {
    if let name = name {
        name
    } else {
        "anon"
    }
}

@StringBuilder
func grade(_ score: Int) -> String {
    if score >= 90 {
        "A"
    } else if score >= 80 {
        "B"
    } else {
        "C"
    }
}
