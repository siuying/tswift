struct Person { var name: String; var age: Int }
let people = [Person(name: "b", age: 30), Person(name: "a", age: 25)]
print(people.map(\.name))
print(people.sorted(by: { $0.age < $1.age }).map(\.name))
let kp = \Person.age
print(people[0][keyPath: kp])
print(type(of: 5), type(of: "s"), type(of: 2.5))
print(Int.self == Int.self)
let m: Any = 42
print(m is Int, m as? Int ?? -1)
@dynamicMemberLookup
struct Config {
  var storage: [String: String]
  subscript(dynamicMember key: String) -> String { storage[key] ?? "" }
}
let cfg = Config(storage: ["host": "localhost"])
print(cfg.host)
