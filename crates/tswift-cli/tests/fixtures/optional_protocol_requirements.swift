// `@objc optional` protocol requirements: a conforming type may implement or
// omit them. Provided implementations are called normally.
@objc protocol Delegate {
    @objc optional func willLoad()
    @objc optional func didLoad(count: Int) -> Int
    @objc optional var badge: Int { get }
    func start()
}

class FullHandler: Delegate {
    var badge: Int { 3 }
    func start() { print("start") }
    func willLoad() { print("willLoad") }
    func didLoad(count: Int) -> Int { count * 2 }
}

let h = FullHandler()
h.start()
h.willLoad()
print(h.didLoad(count: 5))
print(h.badge)

// A type may omit the optional members entirely.
class MinimalHandler: Delegate {
    func start() { print("minimal start") }
}
MinimalHandler().start()

// `optional` remains usable as an ordinary identifier.
var optional = 10
optional += 5
print(optional)

// A non-implementing conformer's optional members resolve to nil through
// optional chaining — methods, labeled methods, and properties.
let handlers: [Delegate] = [MinimalHandler(), FullHandler()]
for d in handlers {
  print(d.didLoad?(count: 5) ?? -1)
  print(d.badge ?? 0)
}

@objc protocol DataSource {
  @objc optional func title(for row: Int) -> String
}
class Bare: DataSource {}
print(Bare().title?(for: 1) ?? "untitled")
