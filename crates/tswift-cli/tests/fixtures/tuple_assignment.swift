// Tuple-destructuring assignment: swaps and the iterative Fibonacci step.
var a = 1, b = 2, c = 3
(a, b, c) = (c, a, b)
print(a, b, c)

var x = 10, y = 20
(x, y) = (y, x)
print(x, y)

func fib(_ n: Int) -> Int {
    var f0 = 0, f1 = 1
    for _ in 0..<n { (f0, f1) = (f1, f0 + f1) }
    return f0
}
print(fib(10))

// Discard with `_`.
var keep = 0
(keep, _) = (7, 99)
print(keep)
