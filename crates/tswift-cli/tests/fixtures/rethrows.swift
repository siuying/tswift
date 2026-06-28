// `rethrows`: the function only throws if the closure argument throws, so a
// non-throwing argument lets the caller skip `try`.
enum MyError: Error { case tooBig }

func applyTwice(_ f: (Int) throws -> Int, to x: Int) rethrows -> Int {
    return try f(f(x))
}

// Non-throwing closure -> no `try` required at the call site.
print(applyTwice({ $0 + 1 }, to: 10))

func doubleOrThrow(_ x: Int) throws -> Int {
    if x > 100 { throw MyError.tooBig }
    return x * 2
}

// Throwing closure -> `try` required, error propagates through `rethrows`.
do {
    print(try applyTwice(doubleOrThrow, to: 5))
    print(try applyTwice(doubleOrThrow, to: 60))
    print("unreached")
} catch {
    print("caught error")
}

// `rethrows` composes: a rethrows wrapper around another rethrows function.
func twice(_ f: () throws -> Int) rethrows -> Int {
    return try f() + f()
}
print(twice { 21 })
