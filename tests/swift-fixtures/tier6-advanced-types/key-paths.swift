// Key-path expressions `\Root.path`, nested paths, inferred-root `\.path`, and
// key paths used as functions must parse and type-check.
// expected-no-diagnostics

struct Address { var city: String }
struct Person { var name: String; var address: Address }

let p = Person(name: "Ada", address: Address(city: "London"))
let kp = \Person.name
let _ = p[keyPath: kp]
let _ = p[keyPath: \Person.address.city]

let people = [p]
let _ = people.map(\.name)
let _ = ["a", "bb"].map(\.count)
