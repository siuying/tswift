// `Self` inside method bodies: as a constructor, for static-member access, and
// for calling static methods of the dynamic type.
struct Counter {
    var value = 0
    static func zero() -> Self { Self() }
    func next() -> Self { Self(value: value + 1) }
}

let c = Counter.zero().next().next()
print(c.value)

struct Factory {
    var id: Int
    static var shared = Factory(id: 9)
    static func make() -> Int { 42 }
    func describe() -> String { "id=\(Self.shared.id) make=\(Self.make())" }
}

print(Factory(id: 5).describe())

class Animal {
    func kind() -> String { "animal" }
    func tag() -> String { "[\(Self.banner())]" }
    class func banner() -> String { "Animal" }
}

class Dog: Animal {
    override func kind() -> String { "dog" }
    override class func banner() -> String { "Dog" }
}

print(Dog().tag())

// A local binding sharing the resolved type name must not shadow the `Self`
// keyword's type resolution.
struct Registry {
    static let count = 7
    func report() -> Int {
        let Registry = 100
        return Registry + Self.count
    }
}

print(Registry().report())
