// A custom `AsyncSequence` driven by `for await`.
struct Countdown: AsyncSequence, AsyncIteratorProtocol {
    var n: Int
    mutating func next() async -> Int? {
        guard n > 0 else { return nil }
        defer { n -= 1 }
        return n
    }
    func makeAsyncIterator() -> Countdown { self }
}

func run() async {
    var sum = 0
    for await x in Countdown(n: 3) { sum += x }
    print("sum \(sum)")
}
run()
