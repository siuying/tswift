// expected-no-diagnostics
// oracle-gap: C msf predates the nonisolated/isolated actor-isolation modifiers

actor Counter {
    var count = 0
    let id = "c1"

    func bump() -> Int {
        count += 1
        return count
    }

    nonisolated func describe() -> String {
        return "counter \(id)"
    }
}

actor Bank {
    var balance = 100
    nonisolated let branch = "main"
    nonisolated(unsafe) var auditCount = 0
}

func report(on counter: isolated Counter) -> Int {
    return counter.count
}
