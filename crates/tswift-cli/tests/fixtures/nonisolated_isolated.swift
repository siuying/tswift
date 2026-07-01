// `nonisolated` members and `isolated` parameters (ADR-0005: the cooperative
// single-threaded executor makes isolation semantically trivial — the
// modifiers are accepted and evaluation is already serialized).

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

func report(on counter: isolated Counter) -> Int {
  return counter.count
}

let c = Counter()
print(c.describe())
Task {
  let n = await c.bump()
  print(n)
  let m = await report(on: c)
  print(m)
}

actor Bank {
  var balance = 100
  nonisolated let branch = "main"
  nonisolated(unsafe) var auditCount = 0

  func deposit(_ n: Int) -> Int {
    balance += n
    return balance
  }
}
let b = Bank()
print(b.branch)
b.auditCount += 1
print(b.auditCount)
Task {
  print(await b.deposit(50))
}
