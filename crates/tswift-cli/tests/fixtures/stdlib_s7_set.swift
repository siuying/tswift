// S7 — Set + copy-on-write.
var unique: Set<Int> = [1, 2, 2, 3]
print(unique.count, unique.isEmpty)
unique.insert(4)
print(unique.contains(2), unique.contains(9))
let r = unique.remove(2)
print(r ?? -1, unique.count)
let a: Set<Int> = [1, 2, 3]
let b: Set<Int> = [2, 3, 4]
print(a.union(b).sorted())
print(a.intersection(b).sorted())
print(a.subtracting(b).sorted())
print(a.symmetricDifference(b).sorted())
print(a.isSubset(of: [1, 2, 3, 4]), a.isSuperset(of: [1, 2]), a.isDisjoint(with: [9]))
print(a.isStrictSubset(of: [1, 2, 3, 4]), a.isStrictSubset(of: [1, 2, 3]))
var m: Set<Int> = [1, 2]
m.formUnion([3, 4])
print(m.sorted())
m.formIntersection([2, 3])
print(m.sorted())
let t = Set([5, 5, 6])
print(t.count)
print(a.sorted())

// Copy-on-write: mutating a copy must not disturb the original.
var orig: Set<Int> = [1, 2, 3]
var cp = orig
cp.insert(99)
print(orig.count, cp.count)
