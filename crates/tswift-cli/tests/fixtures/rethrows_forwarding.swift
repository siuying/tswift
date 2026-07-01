enum E: Error { case bad }

func perform<T>(_ op: () throws -> T) rethrows -> T {
  return try op()
}

print(perform { 41 + 1 })
do {
  _ = try perform { throw E.bad }
} catch {
  print("caught \(error)")
}

func twice(_ body: (Int) throws -> Int) rethrows -> Int {
  return try body(1) + body(2)
}
print(twice { $0 * 10 })
