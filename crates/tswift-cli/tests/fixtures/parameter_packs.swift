func tupleSize<each T>(_ value: repeat each T) -> Int {
  var count = 0
  for _ in repeat each value {
    count += 1
  }
  return count
}
print(tupleSize(1, "two", 3.0))
print(tupleSize())

func describeAll<each T>(_ items: repeat each T) -> String {
  var parts: [String] = []
  for item in repeat each items {
    parts.append("\(item)")
  }
  return parts.joined(separator: ", ")
}
print(describeAll(1, true, "x"))

// Forwarding a pack through another pack-taking function.
func forwarded<each T>(_ items: repeat each T) -> Int {
  return tupleSize(repeat each items)
}
print(forwarded("a", "b", "c", "d"))

// repeat-while statements are unaffected.
var n = 0
repeat {
  n += 1
} while n < 3
print(n)

// Statement-position pack expansion parses (and repeat-while still works,
// re-checked after the statement-dispatch lookahead).
func consumeAll<each T>(_ v: repeat each T) {
  repeat each v
  print("stmt ok", tupleSize(repeat each v))
}
consumeAll(1, 2)
