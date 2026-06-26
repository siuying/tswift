// expected-no-diagnostics
// Tier 2a — nested types: a type declared inside another, referenced by its
// simple name within the enclosing scope and qualified from outside.

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
    func describe() -> Mode {
        return mode
    }
}

class Container {
    class Item {
        let name: String
        init(_ name: String) { self.name = name }
    }
    func make() -> Item {
        return Item("default")
    }
}

let o = Outer(origin: Outer.Point(x: 3, y: 4), mode: .slow)
let p = Outer.Point(x: 7, y: 9)
let item = Container.Item("widget")

let _ = (o.origin.x, p.y, o.describe(), item.name, Container().make().name)
