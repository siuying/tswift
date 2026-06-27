indirect enum Expr {
    case num(Int)
    case add(Expr, Expr)
    case mul(Expr, Expr)
}
func eval(_ e: Expr) -> Int {
    switch e {
    case .num(let n): return n
    case .add(let a, let b): return eval(a) + eval(b)
    case .mul(let a, let b): return eval(a) * eval(b)
    }
}
print(eval(.add(.num(2), .mul(.num(3), .num(4)))))
