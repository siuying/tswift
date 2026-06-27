// Custom `Sequence` / `IteratorProtocol` conformers drive `for-in`.

// A type that is its own iterator.
struct Countdown: Sequence, IteratorProtocol {
    var n: Int
    mutating func next() -> Int? {
        if n <= 0 { return nil }
        defer { n -= 1 }
        return n
    }
}
for x in Countdown(n: 3) { print(x) }

// `where` filtering over a custom sequence.
for x in Countdown(n: 5) where x % 2 == 1 { print("odd \(x)") }

// A separate iterator via makeIterator(), driven lazily so an unbounded
// sequence with `break` terminates.
struct FibIterator: IteratorProtocol {
    var a = 0, b = 1
    mutating func next() -> Int? {
        let r = a
        a = b
        b = r + b
        return r
    }
}
struct Fib: Sequence {
    func makeIterator() -> FibIterator { FibIterator() }
}
var collected: [Int] = []
for x in Fib() {
    if x > 20 { break }
    collected.append(x)
}
print(collected)

// A class conformer (its iterator mutates through the reference).
class Ticker: Sequence, IteratorProtocol {
    var n: Int
    init(_ n: Int) { self.n = n }
    func next() -> Int? {
        if n <= 0 { return nil }
        defer { n -= 1 }
        return n
    }
}
for x in Ticker(3) { print("tick \(x)") }
