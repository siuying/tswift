// expected-no-diagnostics
// Tier 10b/S7 — Set + copy-on-write.

var unique: Set<Int> = [1, 2, 2, 3]
unique.insert(4)
let has = unique.contains(2)
let removed = unique.remove(2)
let c = unique.count
let e = unique.isEmpty

let a: Set<Int> = [1, 2, 3]
let b: Set<Int> = [2, 3, 4]
let u = a.union(b)
let i = a.intersection(b)
let s = a.subtracting(b)
let sd = a.symmetricDifference(b)
let sub = a.isSubset(of: [1, 2, 3, 4])
let sup = a.isSuperset(of: [1, 2])
let dis = a.isDisjoint(with: [9])
let ssub = a.isStrictSubset(of: [1, 2, 3, 4])

var m: Set<Int> = [1, 2]
m.formUnion([3, 4])
m.formIntersection([2, 3])
let made = Set([5, 5, 6])

let _ = (has, removed, c, e, u, i, s, sd, sub, sup, dis, ssub, m, made)
