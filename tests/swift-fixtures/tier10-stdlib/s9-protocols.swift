// expected-no-diagnostics
// Tier 10c/S9 — language-driving protocols & conformance synthesis.

struct Point: Equatable, Comparable, CustomStringConvertible {
    let x: Int
    let y: Int
    static func < (a: Point, b: Point) -> Bool { a.x < b.x }
    var description: String { "(\(x), \(y))" }
}

let p = Point(x: 1, y: 2)
let eq = p == Point(x: 1, y: 2)
let lt = Point(x: 1, y: 0) < Point(x: 2, y: 0)
let s = "\(p)"
let sorted = [Point(x: 2, y: 0), Point(x: 1, y: 0)].sorted()

enum Dir: String, CaseIterable {
    case north, south
}
let all = Dir.allCases
let raw = Dir.north.rawValue
let made = Dir(rawValue: "south")
let same = Dir.north == Dir.north

struct User: Codable, Equatable {
    let name: String
    let age: Int
}
let user = User(name: "Ada", age: 36)

let _ = (eq, lt, s, sorted, all, raw, made, same, user)
