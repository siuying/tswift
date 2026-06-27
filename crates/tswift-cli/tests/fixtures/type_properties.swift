// Type-level (`static`) stored properties and methods on structs and classes.

struct Counter {
    static var total = 0
    static let label = "count"
    var id: Int
    static func bump() {
        total += 1
    }
    static func reset() {
        total = 0
    }
}

Counter.total = 5
print(Counter.total)
print(Counter.label)
Counter.bump()
Counter.bump()
print(Counter.total)
Counter.reset()
print(Counter.total)

// Static storage and a mutating static method on a class.
class Registry {
    static var items: [String] = []
    static func add(_ s: String) {
        items.append(s)
    }
}

Registry.add("a")
Registry.add("b")
Registry.add("c")
print(Registry.items)
print(Registry.items.count)

// A `class` member keyword is also a type property.
struct Config {
    static var verbose = false
}
Config.verbose = true
print(Config.verbose)
