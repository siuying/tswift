import Foundation

// Harden slice 25: Date edge cases
// Ground-truth captured from Swift 6.3.2 on macOS.

let ref = Date(timeIntervalSinceReferenceDate: 0)  // 2001-01-01 00:00:00 UTC

// addingTimeInterval
print(ref.addingTimeInterval(86400).timeIntervalSinceReferenceDate)   // 86400.0

// Equality and ordering
let d1 = Date(timeIntervalSinceReferenceDate: 1000)
let d2 = Date(timeIntervalSinceReferenceDate: 1000)
print(d1 == d2)  // true
print(d1 < d2)   // false
print(d1 > d2)   // false

// compare method
let d3 = Date(timeIntervalSinceReferenceDate: 100)
let d4 = Date(timeIntervalSinceReferenceDate: 200)
print(d3.compare(d4) == .orderedAscending)   // true
print(d4.compare(d3) == .orderedDescending)  // true
print(d3.compare(d3) == .orderedSame)        // true

// timeIntervalSince
print(d4.timeIntervalSince(d3))  // 100.0

// Negative timeIntervalSinceReferenceDate (before 2001)
print(Date(timeIntervalSinceReferenceDate: -86400).timeIntervalSinceReferenceDate)  // -86400.0

// timeIntervalSince1970 — Unix epoch is 978307200 seconds before the reference date
print(Date(timeIntervalSince1970: 0).timeIntervalSinceReferenceDate)  // -978307200.0

// addingTimeInterval negative
let d5 = Date(timeIntervalSinceReferenceDate: 1000).addingTimeInterval(-500)
print(d5.timeIntervalSinceReferenceDate)  // 500.0

// timeIntervalSince1970 round-trip
let ts: TimeInterval = 1_000_000_000.0
let d6 = Date(timeIntervalSince1970: ts)
print(d6.timeIntervalSince1970)  // 1000000000.0

// distantPast < any recent date
print(Date.distantPast < Date(timeIntervalSinceReferenceDate: 0))  // true

// distantFuture > any recent date
print(Date.distantFuture > Date(timeIntervalSinceReferenceDate: 0))  // true

// Date comparison ordering
let earlier = Date(timeIntervalSinceReferenceDate: 0)
let later   = Date(timeIntervalSinceReferenceDate: 3600)
print(earlier < later)   // true
print(later > earlier)   // true
print(earlier <= earlier) // true
