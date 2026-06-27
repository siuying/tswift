// Implicit member expressions `.foo` resolved from the contextual type:
// enum cases and `static` properties.

enum Direction {
    case north, south, east, west
}

func describe(_ d: Direction) -> String {
    switch d {
    case .north: return "N"
    case .south: return "S"
    case .east: return "E"
    case .west: return "W"
    }
}

let heading: Direction = .east
print(describe(heading))
print(describe(.north))

struct Color {
    let r: Int
    let g: Int
    let b: Int
    static let red = Color(r: 255, g: 0, b: 0)
    static let black = Color(r: 0, g: 0, b: 0)
}

func brightness(_ c: Color) -> Int {
    return c.r + c.g + c.b
}

let fill: Color = .red
print(brightness(fill))
print(brightness(.black))
print(Color.red.r)
