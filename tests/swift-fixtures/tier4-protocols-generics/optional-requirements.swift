// `@objc optional` protocol requirements parse and type-check; conforming types
// may implement or omit them.
// expected-no-diagnostics

@objc protocol Delegate {
    @objc optional func willLoad()
    @objc optional var badge: Int { get }
    @objc optional subscript(index: Int) -> Int { get }
    func start()
}

class Handler: Delegate {
    func start() {}
    func willLoad() {}
}

let _ = Handler()
