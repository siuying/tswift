import Foundation

// Concatenation: + and +=
var a = IndexPath(indexes: [1, 2])
let b = IndexPath(indexes: [3, 4])
let c = a + b
print(c)
a += b
print(a)

// compare(_:) -> ComparisonResult
let x = IndexPath(indexes: [1, 2])
let y = IndexPath(indexes: [1, 3])
print(x.compare(y) == .orderedAscending)
print(x.compare(x) == .orderedSame)
print(y.compare(x) == .orderedDescending)

// description / debugDescription
print(x.description)
print(x.debugDescription)

// makeIterator / for-in
var sum = 0
for idx in x { sum += idx }
print(sum)

// subscript get and set
var ip = IndexPath(indexes: [10, 20, 30])
print(ip[0])
ip[0] = 99
print(ip[0])

// index(after:) / index(before:)
print(ip.index(after: 0))
print(ip.index(before: 2))

// range subscript
let sub = ip[1..<3]
print(sub)

// encode (Codable)
let enc = try! JSONEncoder().encode(x)
print(String(data: enc, encoding: .utf8)!)

// decode round-trip
let dec = try! JSONDecoder().decode(IndexPath.self, from: enc)
print(dec)
print(dec == x)

// decode failure: {} has no "indexes" key → keyNotFound (nil from try?)
let bad = Data([123, 125])  // UTF-8 bytes for "{}"
let failed = try? JSONDecoder().decode(IndexPath.self, from: bad)
print(failed == nil)
