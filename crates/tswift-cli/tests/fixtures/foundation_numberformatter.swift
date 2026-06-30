import Foundation

// Decimal style with grouping.
var dec = NumberFormatter()
dec.numberStyle = .decimal
print(dec.string(from: 1234567)!)
print(dec.string(from: 1234.5)!)

// Fixed fraction digits.
dec.minimumFractionDigits = 2
dec.maximumFractionDigits = 2
print(dec.string(from: 1234.5)!)
print(dec.string(from: 8)!)

// Currency style (en_US, $).
var cur = NumberFormatter()
cur.numberStyle = .currency
print(cur.string(from: 1234.5)!)
print(cur.string(from: -9.99)!)
print(cur.string(from: 1000000)!)

// Percent style.
var pct = NumberFormatter()
pct.numberStyle = .percent
print(pct.string(from: 0.25)!)
print(pct.string(from: 1.5)!)

// Grouping toggle.
var plain = NumberFormatter()
plain.numberStyle = .decimal
plain.usesGroupingSeparator = false
print(plain.string(from: 1234567)!)

// Round-trip parse.
print(dec.number(from: "1,234.50")!)
print(cur.number(from: "$1,234.50")!)
print(pct.number(from: "25%")!)
print(dec.number(from: "not a number") == nil)
