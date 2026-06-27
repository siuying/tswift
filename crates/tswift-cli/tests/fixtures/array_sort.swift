// Array.sort() — in-place mutating sort (natural order and by: comparator).
var a = [3, 1, 2]
a.sort()
print(a)

var words = ["banana", "apple", "cherry"]
words.sort()
print(words)

var b = [3, 1, 2]
b.sort(by: { $0 > $1 })
print(b)
