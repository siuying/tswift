// String / Substring `indices` and `makeIterator` (Collection/Sequence).

// ---- String.indices --------------------------------------------------------
let s = "Hello"
print(Array(s.indices).count)   // 5
// Iterating indices lets us subscript each position.
for i in s.indices { print(s[i], terminator: "") }
print("")                        // Hello

// ---- Substring.indices (base-relative) -------------------------------------
let start = s.index(s.startIndex, offsetBy: 1)
let end = s.index(s.startIndex, offsetBy: 4)
let sub = s[start..<end]         // "ell"
print(Array(sub.indices).count)  // 3
// First index of a Substring equals its startIndex (base coordinates).
print(sub.indices.first == sub.startIndex)  // true
for j in sub.indices { print(sub[j], terminator: "") }
print("")                        // ell

// ---- String.makeIterator ---------------------------------------------------
for c in s.makeIterator() { print(c, terminator: "-") }
print("")                        // H-e-l-l-o-

// ---- Substring.makeIterator ------------------------------------------------
for c in sub.makeIterator() { print(c, terminator: "|") }
print("")                        // e|l|l|
