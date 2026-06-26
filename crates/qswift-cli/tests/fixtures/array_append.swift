var numbers = [1, 2, 3]
numbers.append(4)
print(numbers)
print(numbers.count)

// Copy-on-write: mutating a copy leaves the original untouched.
var original = [10, 20]
var copy = original
copy.append(30)
print(original)
print(copy)
