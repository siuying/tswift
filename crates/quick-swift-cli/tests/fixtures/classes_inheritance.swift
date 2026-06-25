class Animal {
    var name: String
    init(_ n: String) { name = n }
    func speak() -> String { return "..." }
}
class Dog: Animal {
    override func speak() -> String { return "woof" }
}
class Cat: Animal {
    override func speak() -> String { return "meow" }
}
let animals: [Animal] = [Dog("Rex"), Cat("Tom")]
for a in animals { print("\(a.name): \(a.speak())") }
