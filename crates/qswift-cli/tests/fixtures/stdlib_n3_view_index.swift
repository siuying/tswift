// N3 follow-up — view-internal String.Index navigation (ADR-0006, #116).
// "a😀b": 'a' (1 byte / 1 UTF-16 unit), '😀' astral (4 bytes / 2 units / 1 scalar),
// 'b' (1 byte / 1 unit).
let s = "a😀b"

// UTF-8 view: every byte is a position; walk it.
print("-- utf8 --")
var i = s.utf8.startIndex
while i != s.utf8.endIndex {
    print(s.utf8[i], terminator: " ")
    i = s.utf8.index(after: i)
}
print("")
print(s.utf8.distance(from: s.utf8.startIndex, to: s.utf8.endIndex))
print(s.utf8[s.utf8.index(before: s.utf8.endIndex)])

// UTF-16 view: the astral scalar contributes a surrogate pair.
print("-- utf16 --")
i = s.utf16.startIndex
while i != s.utf16.endIndex {
    print(s.utf16[i], terminator: " ")
    i = s.utf16.index(after: i)
}
print("")
print(s.utf16.distance(from: s.utf16.startIndex, to: s.utf16.endIndex))
let third = s.utf16.index(s.utf16.startIndex, offsetBy: 3)
print(s.utf16[third])
print(s.utf16.index(s.utf16.startIndex, offsetBy: 99, limitedBy: s.utf16.endIndex) == nil)

// unicodeScalars view.
print("-- scalars --")
i = s.unicodeScalars.startIndex
while i != s.unicodeScalars.endIndex {
    print(s.unicodeScalars[i].value, terminator: " ")
    i = s.unicodeScalars.index(after: i)
}
print("")
print(s.unicodeScalars.distance(from: s.unicodeScalars.startIndex, to: s.unicodeScalars.endIndex))
