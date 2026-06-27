// `ExpressibleBy*Literal` conformances: a literal with a user-type annotation
// is built through the type's literal initializer.

struct Stack: ExpressibleByArrayLiteral {
    var items: [Int]
    init(arrayLiteral elements: Int...) { items = elements }
}
let s: Stack = [1, 2, 3]
print(s.items)

struct Tag: ExpressibleByStringLiteral {
    var raw: String
    init(stringLiteral value: String) { raw = value }
}
let t: Tag = "hello"
print(t.raw)

struct Celsius: ExpressibleByIntegerLiteral, ExpressibleByFloatLiteral {
    var degrees: Double
    init(integerLiteral value: Int) { degrees = Double(value) }
    init(floatLiteral value: Double) { degrees = value }
}
let a: Celsius = 100
let b: Celsius = 37.5
print(a.degrees)
print(b.degrees)

struct Flag: ExpressibleByBooleanLiteral {
    var on: Bool
    init(booleanLiteral value: Bool) { on = value }
}
let f: Flag = true
print(f.on)

final class Name: ExpressibleByStringLiteral {
    var text: String
    init(stringLiteral value: String) { text = value }
}
let n: Name = "Ada"
print(n.text)

// Plain literal-typed bindings remain unaffected.
let x: Int = 5
let arr: [Int] = [1, 2, 3]
print(x, arr.count)
