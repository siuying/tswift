// Extensions that add methods and computed properties to builtin types
// (Int, String, Array), including a `mutating` method and a constrained
// (`where`) extension, dispatched on the value-typed receiver.
extension Int {
    func squared() -> Int { self * self }
    var isEven: Bool { self % 2 == 0 }
    mutating func double() { self *= 2 }
}
print(5.squared())
print(4.isEven)
var n = 21
n.double()
print(n)

extension String {
    func shout() -> String { self + "!" }
    var loud: String { self + "!!!" }
}
print("hi".shout())
print("hey".loud)

extension Array {
    // Unqualified `count` resolves to the receiver's builtin property.
    func secondElement() -> Element? { count >= 2 ? self[1] : nil }
    var middle: Element? { isEmpty ? nil : self[count / 2] }
}
print([1, 2, 3].secondElement()!)
print([10, 20, 30].middle!)

extension Array where Element: Numeric {
    func total() -> Element {
        var s: Element = 0
        for x in self { s += x }
        return s
    }
}
print([10, 20, 30].total())
