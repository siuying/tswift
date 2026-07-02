actor Ledger {
  var total = 0
  func add(_ n: Int) -> Int { total += n; return total }
}
enum JobError: Error { case failed }
func mayThrow(_ n: Int) async throws -> Int {
  if n == 3 { throw JobError.failed }
  return n
}
func run() async {
  let ledger = Ledger()
  for i in 1...3 { _ = await ledger.add(i) }
  print(await ledger.total)
  do {
    let sum = try await withThrowingTaskGroup(of: Int.self) { group in
      for i in 1...3 { group.addTask { try await mayThrow(i) } }
      var s = 0
      for try await v in group { s += v }
      return s
    }
    print("sum \(sum)")
  } catch {
    print("group threw \(error)")
  }
  let seq = AsyncStream<Int> { c in
    for i in 1...5 { c.yield(i) }
    c.finish()
  }
  let doubledEvens = await seq.filter { $0 % 2 == 0 }.map { $0 * 10 }.reduce(0, +)
  print(doubledEvens)
}
await run()
