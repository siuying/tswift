struct Outer {
    struct Point {
        let x: Int
        let y: Int
    }
    enum Mode {
        case fast, slow
    }
    var origin = Point(x: 0, y: 0)
    var mode: Mode = .fast
    func describe() -> String {
        switch mode {
        case .fast: return "fast at \(origin.x),\(origin.y)"
        case .slow: return "slow"
        }
    }
}

let o = Outer(origin: Outer.Point(x: 3, y: 4), mode: .slow)
print(o.origin.x, o.origin.y)
print(o.describe())

// Qualified nested-type construction.
let p = Outer.Point(x: 7, y: 9)
print(p.x, p.y)

// Nested type used through a mutating method building values by simple name.
struct Stack {
    struct Node {
        let value: Int
    }
    var nodes: [Node] = []
    mutating func push(_ v: Int) {
        nodes.append(Node(value: v))
    }
}

var s = Stack()
s.push(1)
s.push(2)
s.push(3)
print(s.nodes.count, s.nodes[2].value)

// Nested class.
class Container {
    class Item {
        let name: String
        init(_ name: String) { self.name = name }
    }
}
let item = Container.Item("widget")
print(item.name)
