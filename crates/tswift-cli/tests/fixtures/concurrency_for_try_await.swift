// `for try await` iterates a throwing async sequence (here an
// `AsyncThrowingStream`), propagating thrown errors to the enclosing `do`/`catch`.
func run() async {
    let stream = AsyncThrowingStream(Int.self) { cont in
        cont.yield(1)
        cont.yield(2)
        cont.yield(3)
        cont.finish()
    }
    var sum = 0
    do {
        for try await x in stream {
            sum += x
        }
    } catch {
        print("error")
    }
    print("sum \(sum)")
}
run()
