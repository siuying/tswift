// expected-no-diagnostics
// Tier 10b/S6 — Dictionary + copy-on-write.

var ages = ["ada": 36, "bob": 40]
ages["carol"] = 28
let ada = ages["ada"]
let withDefault = ages["missing", default: 0]
ages["bob"] = nil
let names = ages.keys.sorted()
let vals = ages.values.sorted()
let c = ages.count
let e = ages.isEmpty

let old = ages.updateValue(99, forKey: "ada")
let removed = ages.removeValue(forKey: "carol")

var d1 = ["a": 1, "b": 2]
d1.merge(["b": 20, "c": 3]) { cur, new in cur + new }
let d2 = ["a": 1].merging(["a": 9]) { cur, new in new }
let doubled = ["a": 1].mapValues { $0 * 2 }
let comp = ["a": 1, "b": -1].compactMapValues { $0 > 0 ? $0 : nil }

let fromPairs = Dictionary(uniqueKeysWithValues: [("one", 1)])
let grouped = Dictionary(grouping: [1, 2, 3], by: { $0 % 2 })

let _ = (ada, withDefault, names, vals, c, e, old, removed, d1, d2, doubled, comp, fromPairs, grouped)
