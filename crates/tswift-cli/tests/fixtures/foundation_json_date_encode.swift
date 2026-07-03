import Foundation

struct Event: Codable {
    let name: String
    let at: Date
}

// Fixed dates for deterministic output.
// ref=86400.0  → 2001-01-02T00:00:00Z (Unix 978393600)
// ref=7200.0   → 2001-01-01T02:00:00Z (Unix 978314400)
let dayTwo = Date(timeIntervalSinceReferenceDate: 86400.0)
let twoHours = Date(timeIntervalSinceReferenceDate: 7200.0)

// 1. Default strategy (.deferredToDate): encodes as timeIntervalSinceReferenceDate
let enc1 = JSONEncoder()
let d1 = try enc1.encode(Event(name: "ref", at: dayTwo))
print(String(data: d1, encoding: .utf8)!)

// 2. secondsSince1970: encodes as Unix seconds
var enc2 = JSONEncoder()
enc2.dateEncodingStrategy = JSONEncoder.secondsSince1970
let d2 = try enc2.encode(Event(name: "ref", at: dayTwo))
print(String(data: d2, encoding: .utf8)!)

// 3. millisecondsSince1970: encodes as Unix milliseconds
var enc3 = JSONEncoder()
enc3.dateEncodingStrategy = JSONEncoder.millisecondsSince1970
let d3 = try enc3.encode(Event(name: "ref", at: dayTwo))
print(String(data: d3, encoding: .utf8)!)

// 4. iso8601: encodes as ISO 8601 string (UTC)
var enc4 = JSONEncoder()
enc4.dateEncodingStrategy = JSONEncoder.iso8601
let d4 = try enc4.encode(Event(name: "ref", at: dayTwo))
print(String(data: d4, encoding: .utf8)!)

// 5. iso8601 with a different date (two hours after reference)
var enc5 = JSONEncoder()
enc5.dateEncodingStrategy = JSONEncoder.iso8601
let d5 = try enc5.encode(Event(name: "two", at: twoHours))
print(String(data: d5, encoding: .utf8)!)

// 6. Round-trip with secondsSince1970
var encRT = JSONEncoder()
encRT.dateEncodingStrategy = JSONEncoder.secondsSince1970
let dataRT = try encRT.encode(Event(name: "rt", at: dayTwo))
var decRT = JSONDecoder()
decRT.dateDecodingStrategy = JSONDecoder.secondsSince1970
let eventRT = try decRT.decode(Event.self, from: dataRT)
print(eventRT.name)
print(eventRT.at.timeIntervalSinceReferenceDate)
