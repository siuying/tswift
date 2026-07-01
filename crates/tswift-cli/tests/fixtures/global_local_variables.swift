// TSPL "Global and Local Variables": computed accessors and observers apply
// to global and local variables, not just type members.

// Global computed variable (get-only, shorthand body).
var stored = 10
var doubled: Int { stored * 2 }
print(doubled)

// Global computed variable with get/set; `newValue` adopts the annotated type.
var celsius = 0.0
var fahrenheit: Double {
  get { return celsius * 9 / 5 + 32 }
  set { celsius = (newValue - 32) * 5 / 9 }
}
fahrenheit = 212
print(celsius)
print(fahrenheit)

// Global stored variable with observers (default parameter names).
var score = 0 {
  willSet { print("will set score to \(newValue)") }
  didSet { print("did set score from \(oldValue) to \(score)") }
}
score = 5
score += 2
print(score)

// Custom observer parameter names.
var level = 1 {
  willSet(incoming) { print("incoming \(incoming)") }
  didSet(previous) { print("previous \(previous)") }
}
level = 3

// Writing through a computed global into a struct member round-trips get/set.
struct Point { var x = 0, y = 0 }
var backing = Point()
var origin: Point {
  get { return backing }
  set { backing = newValue }
}
origin.x = 7
print(backing.x)

// Local computed variables and observers inside a function.
func localVars() {
  var local = 4
  var squared: Int { local * local }
  print(squared)
  var tracked = 0 {
    didSet { print("tracked \(oldValue) -> \(tracked)") }
  }
  tracked = 9
  print(tracked)
}
localVars()

// A closure capturing an observed local keeps firing its observers.
func makeCounter() -> () -> Int {
  var count = 0 {
    didSet { print("count is now \(count)") }
  }
  return {
    count += 1
    return count
  }
}
let tick = makeCounter()
print(tick())
print(tick())

// A mutating method on an observed variable fires didSet once, after the
// mutation, and never exposes a transient receiver.
struct Counter2 { var value = 0
  mutating func bump() { value += 1 }
}
var c2 = Counter2() {
  didSet { print("c2 changed to \(c2.value)") }
}
c2.bump()
print(c2.value)

// A computed variable holding a function value is callable directly.
func one() -> Int { return 1 }
var fn: () -> Int { return one }
print(fn())

// A capture list snapshots the computed variable's value at closure creation.
var n = 5
var computedN: Int { n * 10 }
let snap = { [computedN] in print(computedN) }
n = 6
snap()
