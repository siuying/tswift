// `TaskGroup.next()` consumes one finished child at a time, returning `nil`
// once the group is drained — the manual alternative to `for await`.
func run() async {
    let best = await withTaskGroup(of: Int.self) { group -> Int in
        group.addTask { 10 }
        group.addTask { 30 }
        group.addTask { 20 }
        var m = 0
        while let v = await group.next() {
            m = max(m, v)
        }
        return m
    }
    print("max \(best)")

    let count = await withTaskGroup(of: Int.self) { group -> Int in
        for i in 1...4 { group.addTask { i } }
        var n = 0
        while await group.next() != nil { n += 1 }
        return n
    }
    print("count \(count)")
}
run()
