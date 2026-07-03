// Declared-type-aware Optional dispatch (#242): `take()` and `.debugDescription`
// on a present optional route to Optional (not the flattened wrapped type).
// Result bindings are annotated `T?` so their optionality is recoverable and
// their printed form matches real Swift.

// take() on a present optional: returns the value, resets the receiver to nil.
var x: Int? = 5
let t: Int? = x.take()
print(t.debugDescription, x.debugDescription)

// take() on an absent optional: returns nil, leaves it nil.
var y: Int? = nil
let u: Int? = y.take()
print(u.debugDescription, y.debugDescription)

// take() write-back is observable on a second read.
var z: String? = "hi"
let z1: String? = z.take()
let z2: String? = z.take()
print(z1.debugDescription, z2.debugDescription, z.debugDescription)

// debugDescription: present wraps, absent is the string "nil".
let s: String? = "x"
print(s.debugDescription)
let ni: Int? = nil
print(ni.debugDescription)
let di: Int? = 7
print(di.debugDescription)

// debugDescription used inside a larger print.
print("value:", di.debugDescription)

// take() on a struct field: returns the value and nils the stored slot.
struct Box { var v: Int? }
var b = Box(v: 7)
let bt: Int? = b.v.take()
print(bt.debugDescription, b.v.debugDescription)

// take() through a nested member path.
struct Outer { var box: Box }
var o = Outer(box: Box(v: 3))
let ot: Int? = o.box.v.take()
print(ot.debugDescription, o.box.v.debugDescription)

// take() on a class field (declared field type recovered).
class CBox { var w: String? = "hi" }
let cb = CBox()
let cbt: String? = cb.w.take()
print(cbt.debugDescription, cb.w.debugDescription)

// debugDescription on an array element (subscript element type recovered).
var arr: [Int?] = [5, nil]
print(arr[0].debugDescription)
print(arr[1].debugDescription)

// Optional-chained members keep the *wrapped* type's semantics (String's
// debugDescription here: the value is quoted, not wrapped in `Optional(...)`).
let sc: String? = "x"
print(sc?.debugDescription)

// Regression: optional chaining / map / coalescing / force-unwrap unchanged.
var c: [Int]? = [1, 2, 3]
print(c?.count)
let m: Int? = 5
print(m.map { $0 + 1 })
print(m ?? 0)
let f: Int? = 9
print(f!)
let none: Int? = nil
print(none?.description)
