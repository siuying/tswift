// withExtendedLifetime — lifetime extension is a no-op; the body runs and its
// result is returned.

// 1. Zero-parameter body form.
let xs = [10, 20, 30]
let count = withExtendedLifetime(xs) {
    xs.count
}
print(count)                  // 3

// 2. Body that takes the kept value.
let sum = withExtendedLifetime(xs) { kept in
    kept.reduce(0, +)
}
print(sum)                    // 60

// 3. Reference type with a Void body (side effect only).
class Counter { var n = 0 }
let c = Counter()
withExtendedLifetime(c) {
    c.n += 5
}
print(c.n)                    // 5
