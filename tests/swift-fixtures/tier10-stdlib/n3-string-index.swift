// expected-no-diagnostics
// Tier 10/N3 — String.Index, encoding views, and index-based mutation.

var s = "café!"
let i = s.startIndex
let j = s.index(after: i)
let k = s.index(s.startIndex, offsetBy: 3)
let b = s.index(before: s.endIndex)
let d = s.distance(from: s.startIndex, to: s.endIndex)
let ch = s[i]
let sub = s[s.startIndex..<k]
let limited = s.index(s.startIndex, offsetBy: 9, limitedBy: s.endIndex)

s.insert("X", at: s.startIndex)
let removed = s.remove(at: s.startIndex)
s.removeSubrange(s.startIndex..<s.index(s.startIndex, offsetBy: 1))
s.replaceSubrange(s.startIndex..<s.index(s.startIndex, offsetBy: 1), with: "**")

let scalars = s.unicodeScalars.count
let bytes = s.utf8.count
let units = s.utf16.count
let firstByte = s.utf8.first
let firstScalar = s.unicodeScalars.first

let _ = (j, k, b, d, ch, sub, limited, removed, scalars, bytes, units,
         firstByte, firstScalar)
