// Capture lists: `[weak self]` zeroes after dealloc, `guard let self = self`
// and the SE-0345 `if let self` shorthand rebind it, and reassigning the last
// strong reference runs deinit.

class Owner {
  var name = "o"
  var handler: (() -> String)? = nil
  func setup() {
    handler = { [weak self] in
      guard let self = self else { return "gone" }
      return self.name
    }
  }
  deinit { print("deinit \(name)") }
}

var o: Owner? = Owner()
o!.setup()
let h = o!.handler!
print(h())
o = nil
print(h())

// `if let self` shorthand inside a weak-capture closure.
class Worker {
  var id = 42
  var job: (() -> Int)? = nil
  func arm() {
    job = { [weak self] in
      if let self {
        return self.id
      }
      return -1
    }
  }
  deinit { print("worker deinit") }
}
var w: Worker? = Worker()
w!.arm()
let j = w!.job!
print(j())
w = nil
print(j())

// Optional chaining through weak self.
class Node2 {
  var value = 9
  var report: (() -> Void)? = nil
  func arm() {
    report = { [weak self] in
      print(self?.value ?? -1)
    }
  }
  deinit { print("node deinit") }
}
var n: Node2? = Node2()
n!.arm()
let r = n!.report!
r()
n = nil
r()

// `[unowned x]` does not retain: the owner deinits on release; the closure
// remains callable while the referent is alive.
class Owner2 {
  var n = 7
  deinit { print("owner2 gone") }
}
var strong: Owner2? = Owner2()
let peek = { [unowned obj = strong!] in obj.n }
print(peek())
strong = nil

// An explicit value capture `[y = x * 2]` snapshots at creation.
var x = 5
let cc = { [y = x * 2] in y + 1 }
x = 100
print(cc())
