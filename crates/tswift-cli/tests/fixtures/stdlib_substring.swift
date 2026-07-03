// Substring — member coverage (slice 9, revised representation).
// Substring is a VIEW with base-relative indices: s[i..<j].startIndex == i.

let s = "Hello, World!"
let start = s.startIndex        // index 0 in base
let comma = s.index(start, offsetBy: 5)  // index 5 ("Hello" ends here)
let space = s.index(start, offsetBy: 7)  // index 7 ("World" starts here)
let end = s.endIndex            // index 13

// ---- Substring via String.Index subscript -----------------------------------
let sub = s[start..<comma]  // "Hello"
print(sub)                  // Hello

// ---- count / isEmpty --------------------------------------------------------
print(sub.count)            // 5
print(sub.isEmpty)          // false
let empty = s[start..<start]
print(empty.isEmpty)        // true

// ---- first / last -----------------------------------------------------------
print(sub.first ?? "?")     // H
print(sub.last ?? "?")      // o

// ---- lowercased / uppercased ------------------------------------------------
print(sub.lowercased())     // hello
print(sub.uppercased())     // HELLO

// ---- description / hashValue -----------------------------------------------
print(sub.description)      // Hello
let h1 = sub.hashValue
let h2 = sub.hashValue
print(h1 == h2)             // true (stable within run)

// ---- base: returns ORIGINAL full String (base-relative representation) -----
let b = sub.base
print(b)                    // Hello, World!  (the full parent string)
// base is typed as String (same metatype as s)
print(type(of: b) == type(of: s))  // true

// ---- Indices are BASE-RELATIVE: s[space..<end].startIndex == space ----------
let world = s[space..<end]  // "World!"
let wsi = world.startIndex  // must be == space (offset 7 in base)
let wei = world.endIndex    // must be == end   (offset 13 in base)
// Verify by measuring distance from s.startIndex in s
print(s.distance(from: start, to: wsi))  // 7  (space offset in base)
print(s.distance(from: start, to: wei))  // 13 (end offset in base)

// ---- distance on Substring uses base-relative indices ----------------------
print(world.distance(from: wsi, to: wei))  // 6

// ---- index / subscript on Substring ----------------------------------------
let wi2 = world.index(after: wsi)
print(world[wi2])               // o  (second char of "World!")
let wi3 = world.index(before: wei)
print(world[wi3])               // !  (last char)
let wi4 = world.index(wsi, offsetBy: 3)
print(world[wi4])               // l  (4th char, 0-based)

// ---- subscript by Range<Index> → Substring (chaining, base-relative) ------
// s[space..<end][wsi..<wi4] = "Wor" (still in the original base)
let sub2 = world[wsi..<wi4]    // "Wor" (base-relative)
print(sub2)                     // Wor
print(sub2.count)               // 3
// sub2's startIndex is still base-relative (== space = 7)
print(world.distance(from: wsi, to: sub2.startIndex))  // 0 (same point)

// ---- hasPrefix / hasSuffix / contains --------------------------------------
print(world.hasPrefix("Wor"))  // true
print(world.hasSuffix("ld!"))  // true
print(world.contains("orl"))   // true
print(world.contains("xyz"))   // false

// ---- prefix / suffix return Substring views --------------------------------
let wp = world.prefix(5)      // "World" as Substring view
print(wp)                     // World
// prefix's startIndex stays base-relative (still at space=7)
print(world.distance(from: wsi, to: wp.startIndex))  // 0

let ws = world.suffix(1)      // "!" as Substring view
print(ws)                     // !

// ---- split -----------------------------------------------------------------
let csv = s[start..<space]  // "Hello, "
let parts = csv.split(separator: ",")
print(parts.count)           // 2
print(parts[0])              // Hello

// ---- chained slicing -------------------------------------------------------
let chained = world.prefix(4).suffix(2)
print(chained)               // rl  (prefix(4)="Worl", suffix(2)="rl")

// ---- unicode views ---------------------------------------------------------
print(world.utf8.count)        // 6
print(world.utf16.count)       // 6
print(world.unicodeScalars.count) // 6

// ---- isContiguousUTF8 ------------------------------------------------------
print(world.isContiguousUTF8) // true

// ---- characters (returns self) --------------------------------------------
let chars = sub.characters
print(chars)                  // Hello
print(chars.count)            // 5

// ---- makeContiguousUTF8 (no-op mutating) ----------------------------------
var mcsub = s[start..<comma]
mcsub.makeContiguousUTF8()
print(mcsub)                  // Hello (unchanged)

// ---- append (mutating) -----------------------------------------------------
var ms = s[start..<comma]  // "Hello"
ms.append("!")
print(ms)                    // Hello!

// ---- replaceSubrange (mutating) --------------------------------------------
var rs = s[start..<comma]   // "Hello"
let rsi = rs.startIndex
let rse = rs.index(rsi, offsetBy: 3)
rs.replaceSubrange(rsi..<rse, with: "Hey")
print(rs)                    // Heylo

// ---- Non-zero-start slice mutation: detach-on-write semantics ---------------
// s[1..<3] = "el" with base-relative start=1, end=3.
let i1 = s.index(start, offsetBy: 1)  // grapheme offset 1
let i3 = s.index(start, offsetBy: 3)  // grapheme offset 3
var subNZ = s[i1..<i3]      // "el"
print(subNZ)                // el
// Before mutation: startIndex is base-relative (offset 1).
print(s.distance(from: start, to: subNZ.startIndex))  // 1

// replaceSubrange on non-zero-start slice (base-relative range).
let nzRSI = subNZ.startIndex            // make_index(1)
let nzRSE = subNZ.index(nzRSI, offsetBy: 1)  // make_index(2)
subNZ.replaceSubrange(nzRSI..<nzRSE, with: "X")
print(subNZ)                // Xl  ("e" replaced by "X")
// After mutation: detached; startIndex resets to 0.
print(subNZ.startIndex == s.startIndex) // true (both make_index(0))
print(subNZ.distance(from: subNZ.startIndex, to: subNZ.endIndex))  // 2
print(subNZ.base)           // Xl  (fresh backing string)

// Append on non-zero-start slice: also detaches on mutation.
var subNZ2 = s[i1..<i3]    // "el"
subNZ2.append("!")
print(subNZ2)               // el!
print(subNZ2.base)          // el!  (detached)
print(subNZ2.startIndex == s.startIndex)  // true (both make_index(0))

// ---- String(substring) conversion -----------------------------------------
let str: String = String(sub)
print(str)                   // Hello
print(str.count)             // 5

// ---- Chaining operations -----------------------------------------------------
print(world.prefix(5))       // World
print(world.suffix(1))       // !
