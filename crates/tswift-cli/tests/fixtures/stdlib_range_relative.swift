// RangeExpression.relative(to:) on Range and ClosedRange.

let base = Array(0 ..< 100)

let half = (2 ..< 8).relative(to: base)
print(half.lowerBound, half.upperBound, half.count)

// An inclusive range widens to a half-open Range.
let closed = (1 ... 5).relative(to: base)
print(closed.lowerBound, closed.upperBound, closed.count)

// The resolved Range slices the collection as expected.
print(Array(base[half]))
print(Array(base[closed]))
