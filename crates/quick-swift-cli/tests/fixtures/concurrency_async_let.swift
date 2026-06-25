// `async let` spawns child tasks that run and are awaited for their results.
func fetch(_ id: Int) async -> Int { id * 2 }

@main
struct App {
    static func main() async {
        async let a = fetch(1)
        async let b = fetch(2)
        print(await a + await b)
    }
}
