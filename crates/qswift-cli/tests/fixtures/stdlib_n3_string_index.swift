// N3 — String.Index model, encoding views, and index-based mutation (ADR-0006).

// Index navigation + element/range subscript.
var s = "café!"
let i = s.startIndex
let j = s.index(after: i)
print(s[i], s[j])
print(s.distance(from: s.startIndex, to: s.endIndex))
let k = s.index(s.startIndex, offsetBy: 3)
print(s[k])
let b = s.index(before: s.endIndex)
print(s[b])
print(s.index(s.startIndex, offsetBy: 99, limitedBy: s.endIndex) == nil)
print(s[s.startIndex..<s.index(s.startIndex, offsetBy: 3)])

// Index-based mutation (copy-on-write).
s.insert("X", at: s.startIndex)
print(s)
let removed = s.remove(at: s.startIndex)
print(removed, s)
s.removeSubrange(s.startIndex..<s.index(s.startIndex, offsetBy: 2))
print(s)
s.replaceSubrange(s.startIndex..<s.index(s.startIndex, offsetBy: 1), with: "**")
print(s)

// Encoding views share the String.Index space.
let t = "café"
print(t.unicodeScalars.count, t.utf8.count, t.utf16.count)
print(t.utf8.first!)
for byte in t.utf8 { print(byte, terminator: " ") }
print("")
print(Array(t.utf16))
print(t.unicodeScalars.first!.value)
print(t.utf8[t.utf8.startIndex])
