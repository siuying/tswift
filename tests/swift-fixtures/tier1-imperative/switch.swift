// expected-no-diagnostics
// Tier 1c — switch with value/list/range/tuple/where patterns, fallthrough,
// and labeled break/continue.

func describe(_ value: Int) -> String {
    switch value {
    case 0: return "zero"
    case 1, 2, 3: return "small"
    case 4 ... 9: return "medium"
    case let n where n < 0: return "negative \(n)"
    default: return "large"
    }
}

func quadrant(_ point: (Int, Int)) -> String {
    switch point {
    case (0, 0): return "origin"
    case (let x, 0): return "x-axis at \(x)"
    case (0, let y): return "y-axis at \(y)"
    case (let x, let y) where x == y: return "diagonal"
    default: return "general"
    }
}

func findSum(_ target: Int) -> Int {
    var result = -1
    outer: for i in 0 ..< 3 {
        for j in 0 ..< 3 {
            if i + j == target {
                result = i * 10 + j
                break outer
            }
            if j == 2 { continue outer }
        }
    }
    return result
}

func step(_ x: Int) -> String {
    switch x {
    case 1: fallthrough
    case 2: return "one or two"
    default: return "other"
    }
}

let _ = (describe(5), quadrant((0, 3)), findSum(3), step(1))
