// `AsyncStream`: a producer `yield`s elements into a continuation and `finish`es
// the stream; `for await` consumes them. The producer may run inline or from a
// spawned `Task` (drained before consumption on the cooperative executor).
func run() async {
    let stream = AsyncStream(Int.self) { cont in
        for i in 1...3 { cont.yield(i) }
        cont.finish()
    }
    var sum = 0
    for await x in stream { sum += x }
    print("sum \(sum)")

    let deferred = AsyncStream(Int.self) { cont in
        Task {
            cont.yield(10)
            cont.yield(20)
            cont.finish()
        }
    }
    var collected: [Int] = []
    for await x in deferred { collected.append(x) }
    print("collected \(collected)")
}
run()
