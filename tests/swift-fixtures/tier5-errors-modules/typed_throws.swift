// oracle-gap: the C msf does not parse Swift 6 typed throws `throws(E)`.
// Tier 5 — a function whose error type is statically known.

enum ParseError: Error {
    case empty
}

func parseCount(_ s: String) throws(ParseError) -> Int {
    guard !s.isEmpty else { throw ParseError.empty }
    return s.count
}

let parsed = try? parseCount("hello")
let _ = parsed
