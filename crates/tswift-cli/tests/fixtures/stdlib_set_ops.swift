// Set — subtract, formSymmetricDifference, isStrictSuperset.
var a: Set<Int> = [1, 2, 3, 4]
a.subtract([2, 4])
print(a.sorted())

var b: Set<Int> = [1, 2, 3]
b.formSymmetricDifference([3, 4, 5])
print(b.sorted())

let big: Set<Int> = [1, 2, 3, 4]
print(big.isStrictSuperset(of: [1, 2]))
print(big.isStrictSuperset(of: [1, 2, 3, 4]))
