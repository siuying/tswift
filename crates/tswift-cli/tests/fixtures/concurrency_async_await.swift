// `async`/`await` round-trip: async functions call and await other async
// functions; the @main entry point is itself async.
func double(_ x: Int) async -> Int { x * 2 }
func add(_ a: Int, _ b: Int) async -> Int { a + b }

func compute() async -> Int {
    let a = await double(10)
    let b = await double(11)
    return await add(a, b)
}

@main
struct App {
    static func main() async {
        print(await compute())
    }
}
