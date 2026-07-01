// `AsyncStream.makeStream(of:)` — the builder-free factory that returns a
// `(stream, continuation)` pair. A producer yields on the continuation (inline
// or from a spawned `Task`); the reader is consumed with `for await` or the
// async-sequence algorithms.
func run() async {
    let (stream, continuation) = AsyncStream.makeStream(of: Int.self)
    continuation.yield(1)
    continuation.yield(2)
    continuation.finish()
    var out: [Int] = []
    for await x in stream { out.append(x) }
    print("inline \(out)")

    let (deferred, cont) = AsyncStream.makeStream(of: Int.self)
    Task {
        for i in 1...3 { cont.yield(i * 10) }
        cont.finish()
    }
    let total = await deferred.reduce(0, +)
    print("task total \(total)")
}
run()
