// Extra coverage for Array / Dictionary / Optional / Bool core members.
// Verifies: subscript, init, encode, debugDescription (Dict), !, Bool.init(String).

// ── Array.subscript + Array.init ─────────────────────────────────────────────
var a: [Int] = []                    // Array.init (empty)
a = Array(repeating: 0, count: 3)   // Array.init(repeating:count:)
a[1] = 7                             // subscript set
print(a[0], a[1], a[2])             // subscript get  →  0 7 0

// ── Array.encode (via JSONEncoder) ───────────────────────────────────────────
import Foundation
let enc = JSONEncoder()
let aJson = try enc.encode([10, 20, 30])   // encode token
print(String(data: aJson, encoding: .utf8)!)   // [10,20,30]

// ── Dictionary.debugDescription + subscript + init + encode ──────────────────
var d: [String: Int] = Dictionary(uniqueKeysWithValues: [("x", 1), ("y", 2)])
print(d.debugDescription)                      // [x: 1, y: 2]
d["z"] = 3                                     // subscript set
print(d["x"]!, d["z"]!)                        // subscript get  →  1 3
let dJson = try enc.encode(["a": 1, "b": 2])   // encode sorted keys
print(String(data: dJson, encoding: .utf8)!)   // {"a":1,"b":2}

// ── Bool.init(String) + ! + encode ───────────────────────────────────────────
let bt: Bool? = Bool("true")     // Bool.init(String) failable → true
let bf: Bool? = Bool("false")    // → false
let bn: Bool? = Bool("maybe")    // → nil
print(bt!, bf!, bn == nil)       // true false true
print(!false)                    // ! prefix operator  →  true
let bJson = try enc.encode(false)              // Bool.encode
print(String(data: bJson, encoding: .utf8)!)  // false

// ── Optional.init ─────────────────────────────────────────────────────────────
// Optional(x) wraps a value; in the flattened model the result is the value.
let oi = Optional(7)             // Optional.init
let os = Optional("swift")       // Optional.init
print(oi!, os!)                  // 7 swift

// ── Optional.some + none (pattern matching) ───────────────────────────────────
var present: Int? = 9
var absent: Int? = nil
if case .some(let v) = present { print(v) }   // some  →  9
if case .none = absent { print("none") }       // none  →  none

// ── Optional.encode ───────────────────────────────────────────────────────────
let oJson = try enc.encode(present)            // Optional.encode (present → 9)
print(String(data: oJson, encoding: .utf8)!)  // 9
let nJson = try enc.encode(absent)             // Optional.encode (absent → null)
print(String(data: nJson, encoding: .utf8)!)  // null

// ── ContiguousArray.encode ────────────────────────────────────────────────────
let ca: ContiguousArray<Int> = [4, 5, 6]
let caJson = try enc.encode(ca)               // ContiguousArray.encode
print(String(data: caJson, encoding: .utf8)!) // [4,5,6]
