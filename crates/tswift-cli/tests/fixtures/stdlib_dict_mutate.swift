// Dictionary capacity, reserveCapacity, popFirst and removeAll.
var d = ["a": 1, "b": 2]
print(d.capacity >= d.count)
d.reserveCapacity(50)
print(d.count)

var single = ["only": 9]
if let pair = single.popFirst() {
  print(pair.key, pair.value)
}
print(single.isEmpty)

var empty: [String: Int] = [:]
print(empty.popFirst() == nil)

d.removeAll()
print(d.isEmpty)
