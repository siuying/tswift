// Tuple comparison (element-wise equality, lexicographic ordering) and
// double-optional types.
print((1, "a") == (1, "a"), (1, 2) < (1, 3), (2, 1) > (1, 9))
print((1, 2) <= (1, 2), (1, 2) < (1, 2))
print((1, 2) != (1, 3))
let nn: Int?? = 5
print((nn ?? 0) ?? -1)
let outer: Int?? = nil
print((outer ?? 0) ?? -1)
