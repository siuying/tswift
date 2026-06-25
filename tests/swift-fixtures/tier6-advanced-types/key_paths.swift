// oracle-gap: the C msf does not parse key-path expressions `\Root.path`.
// Tier 6 — key paths and key-path-as-function.

struct Person {
    var name: String
    var age: Int
}

let nameKeyPath = \Person.name
let ada = Person(name: "Ada", age: 36)
let resolvedName = ada[keyPath: nameKeyPath]

let people = [Person(name: "A", age: 1), Person(name: "B", age: 2)]
let names = people.map(\.name)

let _ = (resolvedName, names)
