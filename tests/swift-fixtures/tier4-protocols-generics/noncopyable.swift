// Suppressed constraints `~Copyable` / `~Escapable` parse and type-check on
// types, protocols, and generic parameters.
// expected-no-diagnostics

struct Handle: ~Copyable {
    var fd: Int
}

protocol Resource: ~Copyable {
    var name: String { get }
}

func consumeValue<T: ~Copyable>(_ value: consuming T) -> T { value }

struct Pair<T: ~Copyable & ~Escapable> {
    var first: T
    var second: T
}

let _ = Handle(fd: 1)
let _ = consumeValue(2)
let _ = Pair(first: 3, second: 4)
