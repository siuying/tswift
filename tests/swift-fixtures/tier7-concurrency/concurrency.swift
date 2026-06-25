// oracle-gap: concurrency (async/await/actor/@MainActor) is an F8+ frontend gap;
// the C msf does not fully parse and type these constructs.
// Tier 7 — async functions, await, async let, actors, @MainActor, for await.

func loadValue() async -> Int { 42 }

func loadAll() async -> Int {
    async let a = loadValue()
    async let b = loadValue()
    return await a + b
}

actor Counter {
    private var value = 0
    func increment() { value += 1 }
    func current() -> Int { value }
}

@MainActor
final class ViewModel {
    var title = "Untitled"
}

func sumStream(_ stream: AsyncStream<Int>) async -> Int {
    var total = 0
    for await value in stream {
        total += value
    }
    return total
}

func runSendable(_ work: @Sendable () -> Void) { work() }
