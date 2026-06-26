// expected-no-diagnostics
// Tier 11 / Web demo — Switch: range patterns, where clauses, tuple matching.

func classify(_ n: Int) -> String {
    switch n {
    case ..<0:                                  return "negative"
    case 0:                                     return "zero"
    case 1...9:                                 return "single digit"
    case 10...99:                               return "double digit"
    case let x where x.isMultiple(of: 100):    return "multiple of 100"
    default:                                    return "large"
    }
}

for n in [-3, 0, 7, 42, 300, 1001] {
    print("\(n) → \(classify(n))")
}

// Tuple patterns with where
func quadrant(_ p: (Int, Int)) -> String {
    switch p {
    case (0, 0):                                        return "origin"
    case (let x, 0):                                    return "x-axis @\(x)"
    case (0, let y):                                    return "y-axis @\(y)"
    case (let x, let y) where x > 0 && y > 0:          return "Q1"
    case (let x, let y) where x < 0 && y > 0:          return "Q2"
    case (let x, let y) where x < 0 && y < 0:          return "Q3"
    default:                                            return "Q4"
    }
}

for pt in [(0, 0), (3, 0), (0, 5), (2, 4), (-1, 3), (-2, -2), (1, -1)] {
    print("\(pt) → \(quadrant(pt))")
}
