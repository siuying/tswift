// CollectionOfOne and EmptyCollection members.

// --- CollectionOfOne ---
let one = CollectionOfOne(42)
print(one.count)
print(one.isEmpty)
print(one.first!)
print(one.last!)
print(one.startIndex)
print(one.endIndex)
// for-in
for x in one { print(x) }
// map
let mapped = one.map { $0 * 10 }
print(mapped)
// subscript
print(one[0])
// index
print(one.index(0, offsetBy: 1))
// hashValue consistent
print(one.hashValue == CollectionOfOne(42).hashValue)

// --- EmptyCollection ---
let empty = EmptyCollection<Int>()
print(empty.count)
print(empty.isEmpty)
print(empty.first == nil)
print(empty.last == nil)
print(empty.startIndex)
print(empty.endIndex)
// for-in yields nothing
var seen = false
for _ in empty { seen = true }
print(seen)
// map yields empty array
let emptyMapped = empty.map { $0 + 1 }
print(emptyMapped)
// distance from 0 to 0
print(empty.distance(from: 0, to: 0))
