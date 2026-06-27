// expected-no-diagnostics
// Tier 6 — implicit member expressions (`.foo` / `.foo(args)`) resolved via
// call-site contextual type, disambiguating members two types both declare.

struct Color {
    let name: String
    static let red = Color(name: "red")
    static func custom(_ n: String) -> Color { Color(name: n) }
}

struct Font {
    let name: String
    static let red = Font(name: "red")
    static func custom(_ n: String) -> Font { Font(name: n) }
}

func describe(_ c: Color) -> String { "color:\(c.name)" }

enum Direction { case north, south }
enum Polarity { case north, south }

func step(_ d: Direction) -> String { "\(d)" }

func tally(_ colors: Color...) -> Int { colors.count }

// Static factory disambiguated by the `Color` parameter type.
let factory = describe(.custom("green"))
// Static stored property disambiguated by the `Color` parameter type.
let stored = describe(.red)
// Enum case disambiguated by the `Direction` parameter type.
let dir = step(.north)
// Contextual type propagates into each variadic argument.
let count = tally(.custom("a"), .red)

let _ = (factory, stored, dir, count)
