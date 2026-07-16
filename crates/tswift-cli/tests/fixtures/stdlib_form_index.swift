// Slice — formIndex / index(after:) / index(before:) over Array, ArraySlice,
// ContiguousArray, String and Substring. Indices advance/retreat in place.

// ── Array.index(after:) / index(before:) ─────────────────────────────────────
let a = [10, 20, 30, 40, 50]
var ai = a.startIndex
ai = a.index(after: ai)
print(a[ai])                 // 20
ai = a.index(before: ai)
print(a[ai])                 // 10

// ── Array.formIndex(after:) mutates in place ─────────────────────────────────
var af = a.startIndex
a.formIndex(after: &af)
print(a[af])                 // 20

// ── Array.formIndex(_:offsetBy:) ─────────────────────────────────────────────
a.formIndex(&af, offsetBy: 2)
print(a[af])                 // 40

// ── Array.formIndex(_:offsetBy:limitedBy:) returns Bool ──────────────────────
var al = a.startIndex
let moved = a.formIndex(&al, offsetBy: 2, limitedBy: a.endIndex)
print(moved, a[al])          // true 30
var al2 = a.startIndex
let blocked = a.formIndex(&al2, offsetBy: 99, limitedBy: a.endIndex)
print(blocked, al2 == a.startIndex)  // false true

// ── ArraySlice (base-relative indices) ───────────────────────────────────────
let sl = a[1..<4]            // [20, 30, 40]
var si = sl.startIndex
sl.formIndex(after: &si)
print(sl[si])                // 30

// ── ContiguousArray ──────────────────────────────────────────────────────────
let ca = ContiguousArray([1, 2, 3])
var ci = ca.startIndex
ca.formIndex(&ci, offsetBy: 2)
print(ca[ci])                // 3

// ── String / Substring ───────────────────────────────────────────────────────
let text = "swift"
var ti = text.startIndex
text.formIndex(after: &ti)
print(text[ti])              // w
let sub = text.dropFirst(1)  // "wift"
var ui = sub.startIndex
sub.formIndex(&ui, offsetBy: 2)
print(sub[ui])               // f
