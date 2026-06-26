// N2 follow-up — debugDescription (String(reflecting:)) + collection/Optional
// hashValue, plus the shared-renderer fix that quotes String elements inside
// collections (matching swiftc).

// debugDescription on the CustomDebugStringConvertible builtins.
print((1.5).debugDescription)
print("a\nb\t\"q\"\\".debugDescription)
print([1, 2].debugDescription, ["x", "y"].debugDescription)
print(Set([1]).debugDescription)
print([1: "a"].debugDescription)

// Collections now quote String elements in `print`/`description` too.
print(["a", "b"])
print(["k": "v"])
print(["a", "b"].description)

// Optional `.none` debugDescription (a present optional is unboxed in this
// runtime, so only the nil case surfaces as an Optional receiver).
let n: Int? = nil
print(n.debugDescription)

// Recursive, order-independent hashValue on collections (Swift seeds hashing
// per process, so only within-run consistency is contractual).
print([1, 2, 3].hashValue == [1, 2, 3].hashValue)
print(Set([1, 2]).hashValue == Set([2, 1]).hashValue)
print([1: "a", 2: "b"].hashValue == [2: "b", 1: "a"].hashValue)
