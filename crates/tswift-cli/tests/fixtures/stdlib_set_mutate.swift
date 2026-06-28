// Set capacity, reserveCapacity, removeFirst, popFirst and removeAll.
var s: Set = [1, 2, 3]
print(s.capacity >= s.count)
s.reserveCapacity(50)
print(s.count)

var single: Set = [42]
print(single.removeFirst())
print(single.isEmpty)

var one: Set = [7]
print(one.popFirst() == 7)

var empty: Set<Int> = []
print(empty.popFirst() == nil)

s.removeAll()
print(s.isEmpty)
