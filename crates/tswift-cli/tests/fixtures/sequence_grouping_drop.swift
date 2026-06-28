// Tier 10c/10d — Sequence/Collection completions:
// `Dictionary(grouping:by:)` (trailing-closure and labelled forms) and
// `drop(while:)`.

// Trailing-closure grouping form.
let byFirst = Dictionary(grouping: ["apple", "ant", "bee", "bear"]) { $0.first! }
print(byFirst["a"]!.sorted())
print(byFirst["b"]!.sorted())

// Labelled `by:` form.
let byParity = Dictionary(grouping: 1...6, by: { $0 % 2 })
print(byParity[0]!.sorted())
print(byParity[1]!.sorted())

// `drop(while:)` returns the suffix from the first failing element.
let nums = [1, 2, 3, 4, 1, 2]
print(nums.drop(while: { $0 < 3 }))
print(nums.drop(while: { $0 < 100 }))
print(nums.drop(while: { $0 > 100 }))

// Symmetry with prefix(while:).
print(Array(nums.prefix(while: { $0 < 3 })))

// Empty inputs.
let empty: [Int] = []
print(empty.drop(while: { $0 < 3 }))
let emptyGroups = Dictionary(grouping: empty) { $0 }
print(emptyGroups.count)
