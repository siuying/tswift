// expected-no-diagnostics
// Tier 11 / Web demo — Fibonacci: recursion, for-in, iterative with tuple swap.

func fib(_ n: Int) -> Int {
    if n < 2 { return n }
    return fib(n - 1) + fib(n - 2)
}

print("Fibonacci sequence:")
for i in 0...12 {
    print("  fib(\(i)) = \(fib(i))")
}

// Iterative variant
func fibFast(_ n: Int) -> Int {
    var a = 0
    var b = 1
    for _ in 0..<n {
        let tmp = a + b
        a = b
        b = tmp
    }
    return a
}
print("fibFast(20) = \(fibFast(20))")
