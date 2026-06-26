// S7 — Set methods not exercised elsewhere: subtract, update(with:),
// formSymmetricDifference, isStrictSuperset.
var s: Set<Int> = [1, 2, 3, 4]
s.subtract([2, 4])
print(s.sorted())

var t: Set<Int> = [1, 2, 3]
let replaced = t.update(with: 2)
let inserted = t.update(with: 9)
print(replaced ?? -1, inserted ?? -1, t.sorted())

var u: Set<Int> = [1, 2, 3]
u.formSymmetricDifference([3, 4, 5])
print(u.sorted())

let big: Set<Int> = [1, 2, 3, 4]
print(big.isStrictSuperset(of: [1, 2]))
print(big.isStrictSuperset(of: [1, 2, 3, 4]))
print(big.isStrictSuperset(of: [1, 9]))
// Duplicate elements in the argument sequence must not affect set semantics.
print(big.isStrictSuperset(of: [1, 1, 2, 2]))
