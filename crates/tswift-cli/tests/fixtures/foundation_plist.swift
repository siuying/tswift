import Foundation

// PropertyListEncoder — XML plist encoding.
// Ground-truth: `PropertyListEncoder().encode(X)` with `.xml` format produces
// a valid XML plist with tabs for indentation and alphabetically-sorted keys.

struct Person: Codable {
    let name: String
    let age: Int
}

struct Mixed: Codable {
    let flag: Bool
    let value: Double
    let items: [String]
}

struct Nested: Codable {
    let inner: [String: Int]
}

struct BoolTest: Codable {
    let no: Bool
    let yes: Bool
}

struct DataTest: Codable {
    let other: String
    let payload: Data
}

struct DateTest: Codable {
    let t1: Date
}

var enc = PropertyListEncoder()
enc.outputFormat = .xml

// 1. Simple struct: String + Int (keys sorted: age, name).
let d1 = try enc.encode(Person(name: "Alice", age: 30))
print(String(data: d1, encoding: .utf8)!)

// 2. Mixed types: Bool, Double, [String] (keys sorted: flag, items, value).
let d2 = try enc.encode(Mixed(flag: true, value: 3.14, items: ["a", "b"]))
print(String(data: d2, encoding: .utf8)!)

// 3. Nested dict (keys sorted: inner; inner keys sorted: x, y).
let d3 = try enc.encode(Nested(inner: ["x": 1, "y": 2]))
print(String(data: d3, encoding: .utf8)!)

// 4. Bool values: <false/> and <true/> (keys sorted: no, yes).
let d4 = try enc.encode(BoolTest(no: false, yes: true))
print(String(data: d4, encoding: .utf8)!)

// 5. Data field: base64-encoded in <data> element.
// Argument order matches memberwise initializer (other first, payload second).
let d5 = try enc.encode(DataTest(other: "hi", payload: Data([72, 101, 108, 108, 111])))
print(String(data: d5, encoding: .utf8)!)

// 6. Date field: ISO 8601 in <date> element.
let d6 = try enc.encode(DateTest(t1: Date(timeIntervalSinceReferenceDate: 0)))
print(String(data: d6, encoding: .utf8)!)

// 7. Int array at top level.
let d7 = try enc.encode([1, 2, 3])
print(String(data: d7, encoding: .utf8)!)

// 8. Empty array.
struct EmptyArr: Codable { let items: [String] }
let d8 = try enc.encode(EmptyArr(items: []))
print(String(data: d8, encoding: .utf8)!)
