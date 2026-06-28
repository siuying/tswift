// Tier 10b — empty and typed collection initializers:
// `[T]()`, `[K: V]()`, `Array<T>()`, `Set<T>()`, `Dictionary<K,V>()`, and
// `[T](repeating:count:)`.

var ints = [Int]()
ints.append(1)
ints.append(2)
print(ints)

var map = [String: Int]()
map["a"] = 1
print(map)

var set = Set<Int>()
set.insert(3)
set.insert(3)
print(set.sorted())

let zeros = [Int](repeating: 0, count: 3)
print(zeros)

let emptyArray = Array<Int>()
print(emptyArray.count)

var dict = Dictionary<String, Int>()
dict["x"] = 9
print(dict.count)

// Nested element type.
let grid = [[Int]]()
print(grid.count)

// The sequence-conversion initializers still work.
print(Array(1...3))
print(Set([1, 2, 2, 3]).sorted())
