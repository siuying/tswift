// S5 — randomElement / shuffled are nondeterministic, so verify them by
// asserting invariants that collapse to deterministic output.
let nums = [1, 2, 3, 4, 5]

// randomElement always returns a member of the collection.
let pick = nums.randomElement()!
print(nums.contains(pick))

// randomElement on an empty collection is nil.
let empty: [Int] = []
print(empty.randomElement() == nil)

// shuffled preserves length and multiset, only the order may change.
let mixed = nums.shuffled()
print(mixed.count == nums.count)
print(mixed.sorted() == nums.sorted())

// shuffling a single-element collection is identity.
print([42].shuffled() == [42])

// shuffling an empty collection stays empty.
print(empty.shuffled().count == 0)
