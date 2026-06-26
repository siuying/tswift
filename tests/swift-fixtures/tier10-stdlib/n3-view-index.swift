// expected-no-diagnostics
// Tier 10/N3 follow-up (#116) — view-internal String.Index navigation.

let s = "a😀b"

let bAfter = s.utf8.index(after: s.utf8.startIndex)
let bBefore = s.utf8.index(before: s.utf8.endIndex)
let bDist = s.utf8.distance(from: s.utf8.startIndex, to: s.utf8.endIndex)

let uAfter = s.utf16.index(after: s.utf16.startIndex)
let uOffset = s.utf16.index(s.utf16.startIndex, offsetBy: 3)
let uLimited = s.utf16.index(s.utf16.startIndex, offsetBy: 99, limitedBy: s.utf16.endIndex)
let uDist = s.utf16.distance(from: s.utf16.startIndex, to: s.utf16.endIndex)

let scAfter = s.unicodeScalars.index(after: s.unicodeScalars.startIndex)
let scDist = s.unicodeScalars.distance(from: s.unicodeScalars.startIndex, to: s.unicodeScalars.endIndex)

let _ = (bAfter, bBefore, bDist, uAfter, uOffset, uLimited, uDist, scAfter, scDist)
