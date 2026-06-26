// Classes: inheritance, override, dynamic dispatch
class Animal {
    let name: String
    init(name: String) { self.name = name }
    func sound() -> String { "..." }
    final func describe() -> String { "\(name) says '\(sound())'" }
}

class Dog: Animal {
    override func sound() -> String { "woof" }
}

class Cat: Animal {
    let indoor: Bool
    init(name: String, indoor: Bool) { self.indoor = indoor; super.init(name: name) }
    override func sound() -> String { indoor ? "purr" : "meow" }
}

class Puppy: Dog {
    override func sound() -> String { "yip" }
}

let zoo: [Animal] = [Dog(name: "Rex"), Cat(name: "Whiskers", indoor: true), Puppy(name: "Pip")]
for a in zoo { print(a.describe()) }

// Reference semantics
class Counter {
    var value = 0
    func increment() { value += 1 }
}
let c1 = Counter()
let c2 = c1          // same object
c1.increment(); c1.increment()
print("c1=\(c1.value), c2=\(c2.value)  ← same reference")
print("identical: \(c1 === c2)")
