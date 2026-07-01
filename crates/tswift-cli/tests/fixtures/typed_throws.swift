// Swift 6 typed throws: `throws(E)` on functions, closures, and `do` blocks.

enum ParseError: Error {
  case empty
  case tooLong(Int)
}

func parse(_ s: String) throws(ParseError) -> Int {
  if s.isEmpty { throw ParseError.empty }
  if s.count > 3 { throw ParseError.tooLong(s.count) }
  return s.count
}

do {
  print(try parse("ab"))
  print(try parse(""))
} catch ParseError.empty {
  print("empty")
} catch {
  print("other \(error)")
}

// Propagation between typed-throws functions, with payload patterns.
func wrap() throws(ParseError) -> Int {
  return try parse("toolong")
}
do {
  _ = try wrap()
} catch ParseError.tooLong(let n) {
  print("tooLong \(n)")
}

// `throws(Never)` declares a function that never throws.
func safe() throws(Never) -> Int { return 7 }
print(safe())

// `do throws(E)` blocks.
enum E1: Error { case a }
do throws(E1) {
  throw E1.a
} catch {
  print("caught \(error)")
}

// Typed-throws closures: the parenthesized error type is an effect, not a
// parameter.
enum MyErr: Error { case bad }
do {
  let c = { () throws(MyErr) -> Int in throw MyErr.bad }
  _ = try c()
} catch {
  print("closure \(error)")
}

// Generic error parameters propagate the caller's error type.
func apply<T, E>(_ body: () throws(E) -> T) throws(E) -> T {
  return try body()
}
do {
  let v = try apply { () throws(MyErr) -> Int in 42 }
  print(v)
  _ = try apply { () throws(MyErr) -> Int in throw MyErr.bad }
} catch {
  print("generic \(error)")
}
