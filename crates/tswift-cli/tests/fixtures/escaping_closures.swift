// @escaping closures: stored beyond the call, invoked later, capturing
// locals (by reference) and self (strongly unless a capture list says weak).

var handlers: [() -> Int] = []

func register(_ f: @escaping () -> Int) {
  handlers.append(f)
}

// Escaping closures capture loop bindings per-iteration.
func makeAdders() {
  for i in 1...3 {
    register { i * 10 }
  }
}
makeAdders()
for h in handlers { print(h()) }

// An escaping closure referencing self retains it: releasing the external
// reference does not deinit while the handler array holds the closure.
class Box {
  var v = 1
  func arm() {
    register { self.v + 100 }
  }
  deinit { print("box gone") }
}
var b: Box? = Box()
b!.arm()
print(handlers[3]())
b = nil
print(handlers[3]())

// A weak capture list breaks the retention: deinit runs at b2 = nil.
class Box2 {
  var v = 2
  func arm() {
    register { [weak self] in (self?.v ?? -1) + 200 }
  }
  deinit { print("box2 gone") }
}
var b2: Box2? = Box2()
b2!.arm()
print(handlers[4]())
b2 = nil
print(handlers[4]())

// Escaping closures mutating a captured local observe shared state.
func counterPair() -> (() -> Int, () -> Int) {
  var n = 0
  let bump = { () -> Int in n += 1; return n }
  let read = { () -> Int in n }
  return (bump, read)
}
let (bump, read) = counterPair()
_ = bump()
_ = bump()
print(read())

// completion-handler style: the escaping closure runs after the function
// returned, seeing the arguments it captured.
var completion: ((String) -> Void)? = nil
func fetch(id: Int, then: @escaping (String) -> Void) {
  completion = then
}
fetch(id: 7) { result in print("got \(result)") }
completion?("payload")
