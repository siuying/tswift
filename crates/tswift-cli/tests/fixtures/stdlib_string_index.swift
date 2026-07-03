// S8 — String index-based APIs (String.Index, slicing, mutation).
// String.Index is grapheme-cluster-based, matching Swift semantics.

// ---- startIndex / endIndex / subscript by index ----------------------------
let s = "Hello"
let start = s.startIndex
let end = s.endIndex
print(s[start])              // H
print(s.distance(from: start, to: end))  // 5

// ---- index(after:) / index(before:) ----------------------------------------
let i1 = s.index(after: start)
print(s[i1])                 // e
let iBefore = s.index(before: end)
print(s[iBefore])            // o

// ---- index(_:offsetBy:) ----------------------------------------------------
let i4 = s.index(start, offsetBy: 4)
print(s[i4])                 // o  (H=0,e=1,l=2,l=3,o=4)

// ---- subscript by Range<String.Index> → Substring -------------------------
let sub = s[start..<i4]
print(sub)                   // Hell

// ---- index(_:offsetBy:limitedBy:) → Optional ----------------------------
let limited = s.index(start, offsetBy: 10, limitedBy: end)
print(limited == nil)        // true
let withinLimit = s.index(start, offsetBy: 3, limitedBy: end)
print(withinLimit == nil)    // false

// ---- index equality / ordering ---------------------------------------------
print(start == s.startIndex)   // true
print(i4 < end)                // true

// ---- grapheme-cluster semantics: combining accent --------------------------
let cafe = "cafe\u{301}"       // "café" — 4 grapheme clusters
print(cafe.count)              // 4
let c3 = cafe.index(cafe.startIndex, offsetBy: 3)
print(cafe[c3])                // é  (the whole combining cluster)
print(cafe.distance(from: cafe.startIndex, to: cafe.endIndex))  // 4

// ---- grapheme-cluster semantics: ZWJ family emoji is ONE Character ----------
let fam = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}" // 👨‍👩‍👧‍👦
print(fam.count)               // 1
let famAfter = fam.index(after: fam.startIndex)
print(famAfter == fam.endIndex) // true — one cluster, so after(start)==end
// Stepping with offsetBy: 1 reaches endIndex exactly.
let famEnd = fam.index(fam.startIndex, offsetBy: 1)
print(famEnd == fam.endIndex)  // true
// limitedBy: offsetBy 2 exceeds length 1, returns nil
let famLimited = fam.index(fam.startIndex, offsetBy: 2, limitedBy: fam.endIndex)
print(famLimited == nil)       // true

// ---- mutating: insert(_:at:) -----------------------------------------------
var t = "Hello"
let tIdx = t.index(after: t.startIndex)
t.insert("X", at: tIdx)
print(t)                      // HXello

// ---- mutating: insert(contentsOf:at:) --------------------------------------
var t2 = "Hello"
t2.insert(contentsOf: ", World", at: t2.endIndex)
print(t2)                     // Hello, World

// ---- mutating: remove(at:) → Character -------------------------------------
var u = "Hello"
let removed = u.remove(at: u.startIndex)
print(removed)                // H
print(u)                      // ello

// ---- mutating: removeSubrange(_:) ------------------------------------------
var v = "Hello, World"
let rs = v.index(v.startIndex, offsetBy: 5)
let re = v.index(v.startIndex, offsetBy: 7)
v.removeSubrange(rs..<re)
print(v)                      // HelloWorld

// ---- mutating: replaceSubrange(_:with:) ------------------------------------
var w = "Hello, World"
let wrs = w.index(w.startIndex, offsetBy: 7)
w.replaceSubrange(wrs..<w.endIndex, with: "Swift")
print(w)                      // Hello, Swift
