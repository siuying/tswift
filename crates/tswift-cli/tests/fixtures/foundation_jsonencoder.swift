import Foundation

struct Point: Codable {
    let x: Int
    let y: Int
}

struct Person: Codable {
    let name: String
    let age: Int
    let active: Bool
}

struct Wrapper: Codable {
    let label: String
    let values: [Int]
}

struct Metrics: Codable {
    let score: Double
    let counts: [String: Int]
}

let encoder = JSONEncoder()
let decoder = JSONDecoder()

// Encode a simple two-field struct and print the JSON string
let pointData = try encoder.encode(Point(x: 3, y: 4))
print(String(data: pointData, encoding: .utf8)!)

// Encode a three-field struct with mixed types
let personData = try encoder.encode(Person(name: "Alice", age: 30, active: true))
print(String(data: personData, encoding: .utf8)!)

// Round-trip: decode back and inspect fields
let roundTrip = try decoder.decode(Person.self, from: personData)
print(roundTrip.name)
print(roundTrip.age)
print(roundTrip.active)

// Struct with an array field
let wrapperData = try encoder.encode(Wrapper(label: "nums", values: [1, 2, 3]))
print(String(data: wrapperData, encoding: .utf8)!)

// Struct with a Double field and a Dictionary field
// Dictionary keys are sorted alphabetically for deterministic output
let metricsData = try encoder.encode(Metrics(score: 9.5, counts: ["beta": 2, "alpha": 1]))
print(String(data: metricsData, encoding: .utf8)!)

// Non-finite Double must throw, not emit invalid JSON
do {
    _ = try encoder.encode(Metrics(score: Double.infinity, counts: [:]))
} catch {
    print("non-finite throws")
}
