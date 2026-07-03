// Slice 23 — Dictionary edge-case hardening.
// Ported from Apple's validation-test/stdlib/Dictionary.swift
// (deterministic assertions, no ObjC/bridge/perf tests).

// ── 1. init(uniqueKeysWithValues:) ───────────────────────────────────────────
let pairs = [("a", 1), ("b", 2), ("c", 3)]
let d1 = Dictionary(uniqueKeysWithValues: pairs)
print(d1.count)            // 3
print(d1["a"]!)            // 1
print(d1["c"]!)            // 3

// ── 2. subscript default read — missing key uses default ─────────────────────
let d2: [String: Int] = ["x": 10]
print(d2["z", default: 99])  // 99
print(d2["x", default: 99])  // 10

// ── 3. subscript default write — read-modify-write idiom ────────────────────
var d3: [String: Int] = [:]
d3["key", default: 0] += 1
d3["key", default: 0] += 1
print(d3["key"]!)            // 2

// ── 4. removeValue(forKey:) — present and missing key ────────────────────────
var d4 = ["a": 1, "b": 2]
let removed = d4.removeValue(forKey: "a")
print(removed!)              // 1
print(d4.count)              // 1
let nope = d4.removeValue(forKey: "zzz")
print(nope == nil)           // true

// ── 5. updateValue(_:forKey:) — returns old value ─────────────────────────────
var d5 = ["a": 1]
let old = d5.updateValue(99, forKey: "a")
print(old!)                  // 1
print(d5["a"]!)              // 99
// New key: old value is nil
let new = d5.updateValue(7, forKey: "new")
print(new == nil)            // true
print(d5["new"]!)            // 7

// ── 6. mapValues ─────────────────────────────────────────────────────────────
let d6 = ["x": 1, "y": 2]
let d6m = d6.mapValues { $0 * 2 }
print(d6m["x"]!)             // 2
print(d6m["y"]!)             // 4

// ── 7. filter ────────────────────────────────────────────────────────────────
let d7 = ["a": 1, "b": 2, "c": 3]
let d7f = d7.filter { $0.value > 1 }
print(d7f.count)             // 2
print(d7f["a"] == nil)       // true

// ── 8. merge with collision closure ──────────────────────────────────────────
var d8 = ["a": 1]
d8.merge(["a": 10, "b": 20]) { old, new in old + new }
print(d8["a"]!)              // 11
print(d8["b"]!)              // 20

// ── 9. keys / values ─────────────────────────────────────────────────────────
let d9 = ["x": 10, "y": 20]
print(d9.keys.sorted())      // ["x", "y"]
print(d9.values.sorted())    // [10, 20]

// ── 10. compactMapValues ──────────────────────────────────────────────────────
let d10: [String: String] = ["a": "1", "b": "nope", "c": "3"]
let d10c = d10.compactMapValues { Int($0) }
print(d10c.count)            // 2
print(d10c["a"]!)            // 1

// ── 11. isEmpty and removeAll ─────────────────────────────────────────────────
var d11 = ["a": 1, "b": 2]
print(d11.isEmpty)           // false
d11.removeAll()
print(d11.isEmpty)           // true
print(d11.count)             // 0

// ── 12. description ───────────────────────────────────────────────────────────
print(["k": 42].description)         // ["k": 42]
let empty: [String: Int] = [:]
print(empty.description)             // [:]
