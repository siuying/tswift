// expected-no-diagnostics
// Tier 3 — class inheritance, override, final, super, dynamic dispatch.

class Animal {
    let name: String
    init(name: String) { self.name = name }

    func sound() -> String { "..." }

    final func describe() -> String { "\(name) says \(sound())" }
}

class Dog: Animal {
    override init(name: String) { super.init(name: name) }
    override func sound() -> String { "woof" }
}

class Puppy: Dog {
    override init(name: String) { super.init(name: name) }
    override func sound() -> String { "yip (\(super.sound()))" }
}

let animals: [Animal] = [Dog(name: "Rex"), Puppy(name: "Bit")]
let descriptions = animals.map { $0.describe() }

let _ = descriptions
