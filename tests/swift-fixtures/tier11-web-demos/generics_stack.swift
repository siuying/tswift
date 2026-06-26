// frontend-gap: generic type instantiation 'Stack<Int>()' is not yet parsed —
// the parser misreads 'Stack<Int>' as a comparison expression and reports
// "expected an expression, found RParen" on the '()'.  The same gap exists in
// tier4-protocols-generics/generics.swift (oracle-gap).  The struct declaration
// and constrained extension below are valid Swift 6.
// Remove this file once generic struct instantiation is supported.
//
// Tier 11 / Web demo — Generics: generic Stack (aspirational).

struct Stack<T> {
    var items: [T] = []
    var isEmpty: Bool { items.isEmpty }
    var count: Int { items.count }
    var top: T? { items.last }
    mutating func push(_ item: T) { items.append(item) }
    mutating func pop() -> T? { items.popLast() }
    func display() -> String {
        var parts: [String] = []
        for item in items { parts.append("\(item)") }
        return parts.joined(separator: " → ")
    }
}

var stack = Stack<Int>()
for n in [1, 2, 3, 4, 5] { stack.push(n) }
print("stack: \(stack.display())")
print("top: \(stack.top!), count: \(stack.count)")
print("popped: \(stack.pop()!)")

// Generic free function with Comparable constraint
func largest<T: Comparable>(_ arr: [T]) -> T? {
    guard var best = arr.first else { return nil }
    for x in arr where x > best { best = x }
    return best
}

print("largest int: \(largest([3, 1, 4, 1, 5, 9, 2, 6])!)")
print("largest str: \(largest(["banana", "apple", "cherry"])!)")

// Generic inout swap
func swapValues<T>(_ a: inout T, _ b: inout T) {
    let tmp = a; a = b; b = tmp
}
var x = 10
var y = 99
swapValues(&x, &y)
print("after swap: x=\(x), y=\(y)")
