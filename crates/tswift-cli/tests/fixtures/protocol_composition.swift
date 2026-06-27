protocol Named { var name: String { get } }
protocol Aged { var age: Int { get } }
struct Person: Named, Aged {
    let name: String
    let age: Int
}
func intro(_ p: Named & Aged) -> String {
    return "\(p.name) is \(p.age)"
}
print(intro(Person(name: "Sam", age: 30)))
