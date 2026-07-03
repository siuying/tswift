import Foundation

struct Point: Codable {
    let x: Int
    let y: Int
}

struct Profile: Codable {
    let name: String
    let age: Int
    let score: Double
    let active: Bool
    let nickname: String?
    let tags: [String]
}

struct Envelope: Codable {
    let title: String
    let origin: Point
}

struct IntSeq: Codable {
    let values: [Int]
}

struct PointList: Codable {
    let points: [Point]
}

let decoder = JSONDecoder()
let encoder = JSONEncoder()

// 1. Happy-path round-trip: String/Int/Double/Bool/Optional/Array fields
let p1 = Profile(
    name: "Alice", age: 30, score: 9.5, active: true, nickname: "Al", tags: ["swift", "rust"]
)
let data1 = try encoder.encode(p1)
let p2 = try decoder.decode(Profile.self, from: data1)
print(p2.name)
print(p2.age)
print(p2.score)
print(p2.active)
print(p2.nickname ?? "nil")
print(p2.tags.count)
print(p2.tags[0])
print(p2.tags[1])

// 2. Optional field absent in JSON → nil (no error for Optional)
let json2 = "{\"name\":\"Bob\",\"age\":25,\"score\":7.0,\"active\":false,\"tags\":[]}"
let p3 = try decoder.decode(Profile.self, from: json2)
print(p3.nickname ?? "nil")

// 3. Double field accepts an integer JSON number
let json3 = "{\"name\":\"Carol\",\"age\":20,\"score\":8,\"active\":true,\"tags\":[]}"
let p4 = try decoder.decode(Profile.self, from: json3)
print(p4.score)

// 4. Nested struct round-trip
let env = Envelope(title: "origin", origin: Point(x: 1, y: 2))
let data4 = try encoder.encode(env)
let env2 = try decoder.decode(Envelope.self, from: data4)
print(env2.title)
print(env2.origin.x)
print(env2.origin.y)

// 5. Malformed JSON throws
do {
    _ = try decoder.decode(Point.self, from: "not json")
} catch {
    print("malformed JSON throws")
}

// 6. Int field rejects a fractional JSON number
do {
    _ = try decoder.decode(Point.self, from: "{\"x\":1.5,\"y\":0}")
} catch {
    print("fractional Int throws")
}

// 7. Missing non-optional field throws keyNotFound
do {
    _ = try decoder.decode(Point.self, from: "{\"x\":1}")
} catch {
    print("keyNotFound throws")
}

// 8. [String] array rejects non-string elements (typeMismatch on element)
do {
    _ = try decoder.decode(
        Profile.self,
        from: "{\"name\":\"x\",\"age\":0,\"score\":0.0,\"active\":false,\"tags\":[1]}"
    )
} catch {
    print("[String] rejects non-string element")
}

// 9. [Int] array rejects fractional elements (typeMismatch on element)
do {
    _ = try decoder.decode(IntSeq.self, from: "{\"values\":[1,2.5,3]}")
} catch {
    print("[Int] rejects fractional element")
}

// 10. Registered struct decoded from a scalar JSON value throws typeMismatch
do {
    _ = try decoder.decode(Point.self, from: "42")
} catch {
    print("struct from scalar throws")
}

// 11. Array-typed field receiving scalar JSON throws typeMismatch
do {
    _ = try decoder.decode(IntSeq.self, from: "{\"values\":99}")
} catch {
    print("array field from scalar throws")
}

// 12. [RegisteredStruct] field given a JSON object throws typeMismatch
do {
    _ = try decoder.decode(PointList.self, from: "{\"points\":{\"x\":1,\"y\":2}}")
} catch {
    print("[Point] from object throws")
}

// 13. [RegisteredStruct] field given null throws typeMismatch
do {
    _ = try decoder.decode(PointList.self, from: "{\"points\":null}")
} catch {
    print("[Point] from null throws")
}

// 14. Non-optional registered struct field given null throws valueNotFound
do {
    _ = try decoder.decode(Envelope.self, from: "{\"title\":\"x\",\"origin\":null}")
} catch {
    print("non-optional struct field from null throws")
}
