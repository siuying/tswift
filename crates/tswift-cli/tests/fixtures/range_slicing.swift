// Tier 1 / Tier 10 — collection slicing with two-sided and one-sided ranges.
// Array and String subscripts accept `a..<b`, `a...b`, `a...`, `...b`, `..<b`.

let nums = [10, 20, 30, 40, 50]
print(nums[1..<3])
print(nums[1...3])
print(nums[2...])
print(nums[...2])
print(nums[..<2])

let r = 1..<4
print(nums[r])
print(nums[2...].count)
print(Array(nums[3...]))

let s = "Hello, World"
print(s[0..<5])
print(s[1...3])
print(s[7...])
print(s[...4])
print(s[..<5])

// Slicing through a stored range value works the same.
let mid = 2...4
print(nums[mid])
