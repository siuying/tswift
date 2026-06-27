// Key paths `\Root.path`: reading via `[keyPath:]`, writing through a writable
// key path (including nested paths), key paths as functions in higher-order
// methods, and the identity key path `\.self`.

struct Address { var city: String }
struct Person { var name: String; var age: Int; var address: Address }

var p = Person(name: "Ada", age: 36, address: Address(city: "London"))

// Read through a stored key path and an inline nested key path.
let nameKP = \Person.name
print(p[keyPath: nameKP])
print(p[keyPath: \Person.address.city])

// Write through a writable key path, including a nested path.
p[keyPath: \Person.age] = 40
print(p.age)
p[keyPath: \Person.address.city] = "Paris"
print(p.address.city)

// Key paths as functions in map/filter.
struct Item { var name: String; var active: Bool }
let items = [
    Item(name: "a", active: true),
    Item(name: "b", active: false),
    Item(name: "c", active: true),
]
print(items.map(\.name))
print(items.filter(\.active).map(\.name))

// Computed-property key path.
struct Circle { var radius: Double; var area: Double { radius * radius * 3 } }
print(Circle(radius: 2)[keyPath: \Circle.area])

// Class key path: a reference mutates in place even through a `let` binding.
class Counter { var count: Int; init(_ c: Int) { count = c } }
let counter = Counter(1)
counter[keyPath: \Counter.count] = 7
print(counter[keyPath: \Counter.count])

// Inferred-root key path (`\.count`) and the identity key path (`\.self`).
print(["hi", "hello"].map(\.count))
print([1, 2, 3].map(\.self))
