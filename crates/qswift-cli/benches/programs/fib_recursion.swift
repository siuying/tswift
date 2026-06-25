// Recursion-heavy workload: exponential naive Fibonacci.
// Stresses function call/return, argument passing, and the eval dispatch loop.
func fib(_ n: Int) -> Int {
    return n < 2 ? n : fib(n - 1) + fib(n - 2)
}
print(fib(24))
