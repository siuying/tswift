// Tier 10c — RangeReplaceableCollection mutating operations on Array:
// sequence-flattening append/insert, range removal, predicate removal, and
// in-place reverse.

var a = [1, 2, 3]
a.append(contentsOf: [4, 5])
print(a)

a.insert(contentsOf: [97, 98], at: 1)
print(a)

a.append(contentsOf: 6...8)
print(a)

var b = [1, 2, 3, 4, 5, 6]
b.removeSubrange(1..<3)
print(b)

b.removeAll { $0 % 2 == 0 }
print(b)

var c = [10, 20, 30, 40]
c.reverse()
print(c)

// `append(_:)` on an array of arrays still adds a single element (the
// `contentsOf:` overload is selected only by the argument label).
var nested = [[1], [2]]
nested.append([3])
print(nested)

// `removeAll()` with no predicate still empties the array.
var d = [1, 2, 3]
d.removeAll()
print(d)
