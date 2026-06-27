// S5 — Sequence/Collection algorithm layer (over Array, Range, String).
let nums = [3, 1, 4, 1, 5, 9, 2, 6]
print(nums.map { $0 * 2 })
print(nums.filter { $0 % 2 == 0 })
print(nums.reduce(0) { $0 + $1 })
print(nums.compactMap { $0 > 3 ? $0 : nil })
print([[1, 2], [3], [4, 5]].flatMap { $0 })
print(nums.contains(5), nums.contains(7))
print(nums.allSatisfy { $0 > 0 })
print(nums.first(where: { $0 > 4 }) ?? -1)
print(nums.firstIndex(of: 4) ?? -1)
print(nums.firstIndex(where: { $0 == 5 }) ?? -1)
print(nums.count(where: { $0 > 3 }))
print(nums.sorted())
print(nums.sorted(by: { $0 > $1 }))
print(nums.min() ?? -1, nums.max() ?? -1)
print(Array(nums.reversed()))
print(nums.enumerated().map { "\($0.0):\($0.1)" }.joined(separator: " "))
print(nums.prefix(3))
print(nums.suffix(2))
print(nums.dropFirst())
print(nums.dropLast(2))
print([1, 2, 0, 3, 4, 0, 5].split(separator: 0).map { Array($0) })
print(["a", "b", "c"].joined(separator: "-"))
print([1, 2, 3].elementsEqual([1, 2, 3]), [1, 2].elementsEqual([1, 3]))
print([1, 2, 3, 4].starts(with: [1, 2]))

var total = 0
nums.forEach { total += $0 }
print(total)

// The same algorithms work over a Range receiver.
print((1...5).map { $0 * $0 })
print((1...10).filter { $0 % 3 == 0 })
