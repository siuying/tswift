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

// `do throws(E)` blocks (Swift 6).
do throws(ParseError) {
    _ = try parseCount("")
} catch {
    print("caught \(error)")
}

// Typed-throws closures: `(E)` is part of the effect, not a parameter list.
let closure = { () throws(ParseError) -> Int in
    throw ParseError.empty
}
let _ = try? closure()

// `throws(Never)` declares a non-throwing function.
func total() throws(Never) -> Int { 7 }

// Generic error parameters.
func apply<T, E>(_ body: () throws(E) -> T) throws(E) -> T {
    return try body()
}
