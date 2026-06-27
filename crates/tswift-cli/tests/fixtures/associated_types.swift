// Protocol associated types, including a constrained associated type and a
// generic function that refers to it through `C.Item`.
protocol Container {
    associatedtype Item: Equatable
    var items: [Item] { get }
    func contains(_ x: Item) -> Bool
}

struct Bag<E: Equatable>: Container {
    var items: [E]
    func contains(_ x: E) -> Bool { items.contains(x) }
}

let ints = Bag(items: [1, 2, 3])
print(ints.contains(2))
print(ints.contains(9))

let words = Bag(items: ["a", "b"])
print(words.contains("b"))

// Generic function constrained on the protocol, returning the associated type.
func firstItem<C: Container>(_ c: C) -> C.Item {
    c.items[0]
}
print(firstItem(words))
