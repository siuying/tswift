// `for case` pattern-matching iteration filters and destructures elements.

// Optional pattern: only the non-nil elements are bound.
let maybes: [Int?] = [1, nil, 3, nil, 5]
for case let x? in maybes {
    print("some \(x)")
}

// `for ... where` filters with a Boolean clause.
for i in 0..<6 where i % 2 == 0 {
    print("even \(i)")
}

// Enum-case pattern with an associated-value binding.
enum Shape {
    case circle(Double)
    case square(Double)
}
let shapes: [Shape] = [.circle(1.0), .square(2.0), .circle(3.0)]
for case let .circle(r) in shapes {
    print("circle \(r)")
}

// Tuple pattern combined with a `where` clause.
let pairs = [(1, "a"), (2, "b"), (3, "c")]
for case let (n, s) in pairs where n % 2 == 1 {
    print("\(n)-\(s)")
}
