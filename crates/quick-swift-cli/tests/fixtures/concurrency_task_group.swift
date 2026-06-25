// `withTaskGroup` runs child tasks and aggregates their results with `for await`.
func sumOfSquares(_ n: Int) async -> Int {
    await withTaskGroup(of: Int.self) { group in
        for i in 1...n { group.addTask { i * i } }
        var total = 0
        for await r in group { total += r }
        return total
    }
}

@main
struct App {
    static func main() async {
        print(await sumOfSquares(4))
    }
}
