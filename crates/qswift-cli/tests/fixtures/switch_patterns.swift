func describe(_ p: (Int, Int)) -> String {
    switch p {
    case (0, 0): return "origin"
    case (let x, 0): return "on x at \(x)"
    case (_, let y) where y > 10: return "high y \(y)"
    default: return "elsewhere"
    }
}
print(describe((0, 0)))
print(describe((7, 0)))
print(describe((2, 50)))
print(describe((2, 3)))
