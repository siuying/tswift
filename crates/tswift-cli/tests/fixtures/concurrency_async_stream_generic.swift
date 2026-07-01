// The idiomatic `AsyncStream<Element> { continuation in … }` spelling, using
// the angle-bracket generic argument rather than `AsyncStream(Element.self)`.
func run() async {
    let stream = AsyncStream<Int> { cont in
        for i in 1...3 { cont.yield(i * i) }
        cont.finish()
    }
    var squares: [Int] = []
    for await x in stream { squares.append(x) }
    print(squares)
}
run()
