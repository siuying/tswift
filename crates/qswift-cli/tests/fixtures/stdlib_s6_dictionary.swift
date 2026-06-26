// S6 — Dictionary + copy-on-write.
var ages = ["ada": 36, "bob": 40]
ages["carol"] = 28
print(ages["ada"]!, ages["carol"]!)
print(ages["zzz"] ?? -1)
print(ages.count, ages.isEmpty)

ages["bob"] = nil
print(ages.keys.sorted().joined(separator: ","))
print(ages.values.sorted())

let old = ages.updateValue(99, forKey: "ada")
print(old ?? -1, ages["ada"]!)
let removed = ages.removeValue(forKey: "carol")
print(removed ?? -1, ages.count)
print(ages["missing", default: 0])

// Copy-on-write: mutating a copy must not disturb the original.
var a = ["x": 1]
var b = a
b["y"] = 2
print(a.count, b.count)

var d1 = ["a": 1, "b": 2]
d1.merge(["b": 20, "c": 3]) { cur, new in cur + new }
print(d1["a"]!, d1["b"]!, d1["c"]!)
let d2 = ["a": 1].merging(["a": 9]) { cur, new in new }
print(d2["a"]!)

let doubled = ["a": 1, "b": 2].mapValues { $0 * 2 }
print(doubled["a"]!, doubled["b"]!)
let comp = ["a": 1, "b": -1].compactMapValues { $0 > 0 ? $0 : nil }
print(comp.count, comp["a"]!)

let fromPairs = Dictionary(uniqueKeysWithValues: [("one", 1), ("two", 2)])
print(fromPairs["one"]!, fromPairs["two"]!)
let grouped = Dictionary(grouping: [1, 2, 3, 4, 5], by: { $0 % 2 })
print(grouped[0]!.sorted(), grouped[1]!.sorted())

var total = 0
for pair in ["a": 1, "b": 2, "c": 3] { total += pair.1 }
print(total)
