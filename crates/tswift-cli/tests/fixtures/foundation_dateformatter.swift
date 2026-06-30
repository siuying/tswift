import Foundation

// 2024-06-29 12:34:56 UTC
let date = Date(timeIntervalSince1970: 1719664496.0)

var f = DateFormatter()
f.dateFormat = "yyyy-MM-dd HH:mm:ss"
print(f.string(from: date))

f.dateFormat = "MMMM d, yyyy"
print(f.string(from: date))

f.dateFormat = "EEEE h:mm a"
print(f.string(from: date))

// Round-trip parse with a numeric pattern.
f.dateFormat = "yyyy-MM-dd HH:mm:ss"
let parsed = f.date(from: "2024-06-29 12:34:56")!
print(parsed.timeIntervalSince1970)
print(parsed.timeIntervalSinceReferenceDate == date.timeIntervalSinceReferenceDate)

// Style-based formatting (en_US-ish, UTC).
f.dateFormat = nil
f.dateStyle = .medium
f.timeStyle = .short
print(f.string(from: date))

f.dateStyle = .full
f.timeStyle = .none
print(f.string(from: date))

// Bad input yields nil.
f.dateFormat = "yyyy-MM-dd"
print(f.date(from: "garbage") == nil)
