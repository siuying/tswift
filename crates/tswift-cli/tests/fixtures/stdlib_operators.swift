// Core operators: arithmetic/concatenation +/+= and == across the value types.
var i = 5
i += 3
print(i, i == 8, 2 + 3)

var d = 1.5
d += 0.5
print(d, d == 2.0, 1.0 + 2.0)

var s = "ab"
s += "cd"
print(s, s == "abcd", "x" + "y")

print([1, 2] == [1, 2], [1] + [2])
print(Set([1, 2]) == Set([2, 1]))
print(["a": 1, "b": 2] == ["b": 2, "a": 1])
print((1..<3) == (1..<3), (1...3) == (1..<3))
