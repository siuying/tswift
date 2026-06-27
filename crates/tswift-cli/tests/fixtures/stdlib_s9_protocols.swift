// S9 — language-driving protocols & conformance synthesis.
import Foundation

struct Point: Equatable, Comparable, CustomStringConvertible {
    let x: Int
    let y: Int
    static func < (a: Point, b: Point) -> Bool {
        a.x == b.x ? a.y < b.y : a.x < b.x
    }
    var description: String { "(\(x), \(y))" }
}

let p = Point(x: 1, y: 2)
let q = Point(x: 1, y: 2)
print(p == q, p == Point(x: 9, y: 9))
print(p)
print("point is \(p)")

let pts = [Point(x: 3, y: 0), Point(x: 1, y: 5), Point(x: 1, y: 2)]
print(pts.sorted().map { $0.description }.joined(separator: " "))
print(pts.min()!.description, pts.max()!.description)

enum Dir: String, CaseIterable {
    case north, south, east, west
}
print(Dir.allCases.count)
print(Dir.allCases.map { $0.rawValue }.joined(separator: ","))
print(Dir.north == Dir.north, Dir.north == Dir.south)
print(Dir(rawValue: "east")?.rawValue ?? "none")

struct User: Codable, Equatable {
    let name: String
    let age: Int
}
let user = User(name: "Ada", age: 36)
let data = try! JSONEncoder().encode(user)
let back = try! JSONDecoder().decode(User.self, from: data)
print(back.name, back.age)
print(user == back)
