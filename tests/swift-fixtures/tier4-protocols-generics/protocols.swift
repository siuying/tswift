// expected-no-diagnostics
// Tier 4a — protocol declaration, default impls, inheritance, composition,
// associated types, existentials, class-only protocols.

protocol Named {
    var name: String { get }
    func greet() -> String
}

extension Named {
    func greet() -> String { "Hi, I'm \(name)" }
}

protocol Aged {
    var age: Int { get }
}

typealias Person = Named & Aged

struct Employee: Named, Aged {
    let name: String
    let age: Int
}

protocol Startable {
    func start()
}

protocol Drivable: Startable {
    func drive()
}

struct Sedan: Drivable {
    func start() {}
    func drive() {}
}

protocol Container {
    associatedtype Item
    var count: Int { get }
    func item(at index: Int) -> Item
}

struct IntBox: Container {
    let items: [Int]
    var count: Int { items.count }
    func item(at index: Int) -> Int { items[index] }
}

protocol Cloneable: AnyObject {
    func clone() -> Self
}

func describe(_ value: any Named) -> String { value.greet() }

let employee = Employee(name: "Ada", age: 36)
let person: Person = employee
let box = IntBox(items: [1, 2, 3])
let sedan = Sedan()
sedan.drive()

let _ = (describe(employee), person.name, person.age, box.item(at: 0), box.count)
