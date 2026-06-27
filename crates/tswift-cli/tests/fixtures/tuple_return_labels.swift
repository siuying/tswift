// Element labels from a function's tuple return type apply to the returned
// value, so `.lo`/`.hi` work even when the return expression was unlabeled.

func bounds() -> (lo: Int, hi: Int) { return (2, 8) }
let b = bounds()
print(b.lo)
print(b.hi)
print(bounds().hi)

func minmax(_ xs: [Int]) -> (min: Int, max: Int) {
    return (xs.min()!, xs.max()!)
}
let r = minmax([3, 1, 4, 1, 5])
print(r.min)
print(r.max)

// Unlabeled tuple returns still use positional access.
func plain() -> (Int, Int) { return (4, 5) }
let p = plain()
print(p.0)
print(p.1)
