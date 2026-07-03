// Type-directed optional printing (#241): present elements of an optional-typed
// value render as `Optional(...)`, absent ones as `nil` — matching Swift.

// Direct literal argument.
print([Optional("x"), nil])

// Annotated binding.
let a: [String?] = ["x", nil]
print(a)

// Un-annotated literal-inferred binding.
let b = [Optional("x"), nil]
print(b)

// Every scalar element type.
print([Optional(1), nil])
print([Optional(2.5), nil])
print([Optional(true), nil])

// Nested arrays.
print([[Optional("x"), nil]])

// Dictionary optional values: absent and present.
let d: [String: Int?] = ["a": nil]
print(d)
let dp: [Int: String?] = [1: "x"]
print(dp)

// An identifier with an optional declared type infers `[T?]` for the literal.
let x: String? = "x"
print([x, nil])

// debugPrint shows the wrapper too.
debugPrint([Optional("x"), nil])

// Separator interplay: only the optional-typed argument is rewritten.
print([Optional(1), nil], [1, 2], separator: " | ")

// Top-level scalar optionals: present wraps, absent is nil (Swift fidelity).
let s: String? = "x"
print(s)
let n: Int? = nil
print(n)
debugPrint(s)

// `terminator:` label is filtered out and does not disturb type alignment.
print(s, terminator: "!\n")

// No regression: non-optional collections and scalars print unchanged.
print(["x"])
print([1, 2, 3])
print("plain")
