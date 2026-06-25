// oracle-gap: the C msf does not resolve a generic type parameter (`Element`)
// inside the member signatures of its owning generic type.
// Tier 4b — generic functions, constraints, generic types, generic subscripts,
// contextual where on extensions.

func swapValues<T>(_ a: inout T, _ b: inout T) {
    let tmp = a
    a = b
    b = tmp
}

func maxOf<T: Comparable>(_ a: T, _ b: T) -> T { a > b ? a : b }

func allEqual<T: Equatable>(_ values: [T]) -> Bool {
    guard let first = values.first else { return true }
    for value in values where value != first { return false }
    return true
}

struct Stack<Element> {
    private var items: [Element] = []
    var count: Int { items.count }
    mutating func push(_ item: Element) { items.append(item) }
    mutating func pop() -> Element? { items.popLast() }
    subscript(_ index: Int) -> Element { items[index] }
}

extension Stack where Element: Equatable {
    func contains(_ item: Element) -> Bool {
        for i in 0 ..< count where self[i] == item { return true }
        return false
    }
}

var stack = Stack<Int>()
stack.push(1)
stack.push(2)
let popped = stack.pop()

var a = 1
var b = 2
swapValues(&a, &b)

let _ = (maxOf(3, 7), allEqual([1, 1, 1]), stack.count, popped, a, b, stack.contains(1))
