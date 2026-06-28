// Tier 3a — closure shorthand arguments destructuring a single tuple element.
// A closure that references `$1`, `$2`, … but receives one tuple argument
// binds the shorthands to the tuple's elements (Swift's tuple-splat shorthand).

let pairs = [(1, "a"), (2, "b"), (3, "c")]
print(pairs.map { "\($0):\($1)" })

// `enumerated()` yields `(offset, element)` tuples.
let letters = ["x", "y", "z"]
print(letters.enumerated().map { "\($0)=\($1)" })

// Dictionary elements are `(key, value)` tuples.
let scores = ["a": 1, "b": 2]
print(scores.map { "\($0)->\($1)" }.sorted())

// `zip` yields pair tuples.
print(zip([1, 2, 3], [10, 20, 30]).map { $0 + $1 })

// A 3-tuple splats across `$0`, `$1`, `$2`.
let triples = [(1, 2, 3), (4, 5, 6)]
print(triples.map { $0 + $1 + $2 })

// A closure using only `$0` keeps the whole tuple (accessed via `.0`/`.1`).
print(pairs.map { $0.0 })

// A literal `$1` in a non-interpolated position is not a shorthand reference.
print([10, 20].map { "$1 unit: \($0)" })
