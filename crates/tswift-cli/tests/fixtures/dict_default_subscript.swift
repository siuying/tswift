// `dict[key, default: d]` compound assignment — a missing key reads the
// default before applying the operator, the idiomatic frequency-count pattern.

var counts: [String: Int] = [:]
for w in ["a", "b", "a", "c", "a", "b"] {
    counts[w, default: 0] += 1
}
print(counts["a"]!)
print(counts["b"]!)
print(counts["c"]!)
print(counts["z", default: 99])
