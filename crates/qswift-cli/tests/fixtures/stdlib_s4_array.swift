// S4 — Array intrinsics + copy-on-write.
var a = [1, 2, 3]
a.insert(0, at: 0)
print(a)
let removed = a.remove(at: 2)
print(removed, a)
print(a.removeLast(), a)
print(a.removeFirst(), a)
a.removeAll()
print(a, a.isEmpty)

var b = [10, 20, 30]
print(b.count, b.first ?? -1, b.last ?? -1, b.startIndex, b.endIndex)
b[1] = 99
print(b)
b.reserveCapacity(100)
print(b.count, b.capacity >= 0)

print([1, 2] + [3, 4])
var c = [1]
c += [2, 3]
print(c)
print([1, 2] == [1, 2], [1, 2] == [1, 3])

let z = Array(repeating: 0, count: 3)
print(z)
let fromRange = Array(1...4)
print(fromRange)

// Copy-on-write: a mutated copy must not disturb the original.
var original = [1, 2, 3]
var copy = original
copy.insert(99, at: 0)
copy.removeLast()
print(original)
print(copy)
