func inc(_ n: Int) -> Int { return n + 1 }
func twice(_ f: (Int) -> Int, _ x: Int) -> Int { return f(f(x)) }
let g = inc
print(g(10))
print(twice(inc, 5))
