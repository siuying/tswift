// Implicit member expressions resolving to static properties and static
// factory methods in a contextual (known) type position.
struct Color {
    let name: String
    static let red = Color(name: "red")
    static let blue = Color(name: "blue")
    static func custom(_ n: String) -> Color { Color(name: n) }
    static func mix(_ a: String, _ b: String) -> Color { Color(name: a + "+" + b) }
}

func describe(_ c: Color) -> String { c.name }

// Static property via implicit member.
let c: Color = .red
print(describe(c))
print(describe(.blue))

// Static factory method via implicit member.
print(describe(.custom("green")))
print(describe(.mix("red", "blue")))

// Class with a static factory used as an implicit member.
class Shape {
    let sides: Int
    init(sides: Int) { self.sides = sides }
    static func triangle() -> Shape { Shape(sides: 3) }
}
func count(_ s: Shape) -> Int { s.sides }
print(count(.triangle()))
