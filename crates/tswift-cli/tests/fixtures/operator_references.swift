// Bare operators passed as function values to higher-order functions.
let nums = [1, 2, 3, 4, 5]
print(nums.reduce(0, +))
print(nums.reduce(1, *))
print(nums.sorted(by: >))

let words = ["banana", "apple", "cherry"]
print(words.sorted(by: <))
print([10, 5, 8].max(by: <)!)
