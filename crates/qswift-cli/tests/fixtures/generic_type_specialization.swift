// Explicit generic type arguments at a call / member site:
// `Type<Args>(…)`, `Type<Args>()`, and `Type<Args>.member`. The runtime infers
// the concrete element types from values, so the `<…>` clause is parsed and
// discarded rather than treated as a comparison chain (`Stack < Int > (…)`).
struct Stack<T> {
    var items: [T] = []
    mutating func push(_ x: T) { items.append(x) }
    func peek() -> T? { items.last }
}

// Explicit type args with labeled initializer arguments.
var a = Stack<Int>(items: [1, 2, 3])
print(a.items.count)

// Explicit type args with an empty argument list.
var b = Stack<Int>()
b.push(9)
b.push(4)
print(b.peek()!)
print(b.items.count)

// Extension on a generic type, including a constrained (`where`) extension.
extension Stack {
    var depth: Int { items.count }
}
extension Stack where T: Equatable {
    func has(_ x: T) -> Bool { items.contains(x) }
}
print(a.depth)
print(a.has(2))

// Static method reached through explicit type arguments.
struct Factory<T> {
    static func tag() -> String { "factory" }
}
print(Factory<String>.tag())

// Comparison operators must still parse as comparisons.
let x = 3, y = 5
print(x < y && y > x)
