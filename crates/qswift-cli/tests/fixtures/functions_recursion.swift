func factorial(_ n: Int) -> Int {
    return n == 0 ? 1 : n * factorial(n - 1)
}
func fib(_ n: Int) -> Int {
    return n < 2 ? n : fib(n - 1) + fib(n - 2)
}
print(factorial(5))
print(fib(10))
