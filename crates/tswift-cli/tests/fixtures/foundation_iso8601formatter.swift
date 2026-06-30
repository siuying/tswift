import Foundation

// 2024-06-29 12:00:00 UTC
let date = Date(timeIntervalSince1970: 1719662400.0)

let iso = ISO8601DateFormatter()
print(iso.string(from: date))

let parsed = iso.date(from: "2024-06-29T12:00:00Z")!
print(parsed.timeIntervalSince1970)
print(parsed.timeIntervalSinceReferenceDate == date.timeIntervalSinceReferenceDate)

// Round-trip an epoch instant.
let epoch = Date(timeIntervalSince1970: 0.0)
print(iso.string(from: epoch))

// Reference date.
let reference = Date(timeIntervalSinceReferenceDate: 0.0)
print(iso.string(from: reference))

// Unparseable input yields nil.
print(iso.date(from: "not a date") == nil)
