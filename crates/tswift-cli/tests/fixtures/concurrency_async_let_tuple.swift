// Awaiting a tuple of `async let` bindings drives each child and destructures
// the results: `let (x, y) = await (a, b)`.
func fetch(_ id: Int) async -> Int { id * 10 }

func run() async {
    async let a = fetch(1)
    async let b = fetch(2)
    async let c = fetch(3)
    let (x, y) = await (a, b)
    print("pair \(x) \(y)")
    let total = await (x + (await c))
    print("total \(total)")
}
run()
