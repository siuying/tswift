// `required` initializers must be implemented by every subclass; the runtime
// runs them through the inheritance chain.
class Animal {
    let name: String
    required init(name: String) {
        self.name = name
    }
    func describe() -> String {
        "animal \(name)"
    }
}

class Dog: Animal {
    required init(name: String) {
        super.init(name: name)
    }
    override func describe() -> String {
        "dog \(name)"
    }
}

let a = Animal(name: "thing")
let d = Dog(name: "Rex")
print(a.describe())
print(d.describe())
