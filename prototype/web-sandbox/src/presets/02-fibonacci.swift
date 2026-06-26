// Classic fibonacci — recursion + for-in
func fib(_ n: Int) -> Int {
    if n < 2 { return n }
    return fib(n - 1) + fib(n - 2)
}

print("Fibonacci sequence:")
for i in 0...12 {
    print("  fib(\(i)) = \(fib(i))")
}

// Iterative variant with tuple-swap assignment
func fibFast(_ n: Int) -> Int {
    var a = 0, b = 1
    for _ in 0..<n {
        (a, b) = (b, a + b)
    }
    return a
}
print("fibFast(20) = \(fibFast(20))")
