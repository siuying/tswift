func work(_ n: Int) async -> Int { n * 10 }
func run() async {
  async let a = work(1)
  async let b = work(2)
  let (x, y) = await (a, b)
  print(x + y)
  let total = await withTaskGroup(of: Int.self) { group in
    for i in 1...4 { group.addTask { await work(i) } }
    var sum = 0
    for await v in group { sum += v }
    return sum
  }
  print(total)
  let t = Task { () -> Int in
    try? await Task.sleep(nanoseconds: 1)
    return 7
  }
  print(await t.value)
  let cancelled = Task { () -> String in
    if Task.isCancelled { return "cancelled" }
    return "ran"
  }
  cancelled.cancel()
  print(await cancelled.value)
  let stream = AsyncStream<Int> { c in
    c.yield(1); c.yield(2); c.finish()
  }
  var got: [Int] = []
  for await v in stream { got.append(v) }
  print(got)
  let answer: Int = await withCheckedContinuation { k in
    k.resume(returning: 99)
  }
  print(answer)
}
await run()
