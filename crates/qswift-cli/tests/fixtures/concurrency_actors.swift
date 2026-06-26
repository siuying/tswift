// `actor` serializes access to its mutable state; `@MainActor` isolation runs on
// the cooperative main executor; a `Sendable` value crosses an await boundary.
actor Counter {
    private var value = 0
    func increment() { value += 1 }
    func get() -> Int { value }
}

struct Tally: Sendable { let label: String }

@MainActor
func report(_ tally: Tally, _ n: Int) {
    print("\(tally.label) = \(n)")
}

func run() async {
    let counter = Counter()
    for _ in 1...3 { await counter.increment() }
    let n = await counter.get()
    await report(Tally(label: "count"), n)
}
run()
