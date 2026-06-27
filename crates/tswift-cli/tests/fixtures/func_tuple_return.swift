// Multiple return values via tuples (positional access), @discardableResult,
// and a Never-returning function used to exit a guard.

func minMax(_ a: [Int]) -> (Int, Int) {
    var mn = a[0]
    var mx = a[0]
    for v in a {
        if v < mn { mn = v }
        if v > mx { mx = v }
    }
    return (mn, mx)
}

let bounds = minMax([3, 1, 4, 1, 5, 9, 2])
print(bounds.0, bounds.1)

let (lo, hi) = minMax([8, 2, 7])
print(lo, hi)

@discardableResult
func record(_ message: String) -> Int {
    return message.count
}

record("ignored result is fine")
let kept = record("kept")
print(kept)

func fail(_ message: String) -> Never {
    fatalError(message)
}

func doubleIfPositive(_ x: Int) -> Int {
    guard x > 0 else { fail("must be positive") }
    return x * 2
}

print(doubleIfPositive(21))
