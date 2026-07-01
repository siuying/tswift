// `AsyncSequence` algorithms (`map`/`filter`/`reduce`/`contains`/`first`/
// `prefix`) over a custom async sequence. The cooperative executor materialises
// the producer, so these compose like their synchronous counterparts.
struct Nums: AsyncSequence, AsyncIteratorProtocol {
    var n = 0
    mutating func next() async -> Int? {
        guard n < 5 else { return nil }
        defer { n += 1 }
        return n
    }
    func makeAsyncIterator() -> Nums { self }
}

func run() async {
    let doubled = await Nums().map { $0 * 2 }
    print("map \(doubled)")
    let evens = await Nums().filter { $0 % 2 == 0 }
    print("filter \(evens)")
    let total = await Nums().reduce(0) { $0 + $1 }
    print("reduce \(total)")
    let has3 = await Nums().contains { $0 == 3 }
    print("contains \(has3)")
    let firstBig = await Nums().first { $0 > 2 }
    print("first \(firstBig ?? -1)")
    let firstTwo = await Nums().prefix(2)
    print("prefix \(firstTwo)")
}
run()
