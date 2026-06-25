// expected-no-diagnostics
// Tier 1c — if/else, if-expression, guard, while/repeat, for-in, for-where.

let score = 75

if score >= 90 {
    print("A")
} else if score >= 70 {
    print("B")
} else {
    print("C")
}

let grade = if score >= 70 { "pass" } else { "fail" }

func firstPositive(_ xs: [Int]) -> Int? {
    for x in xs where x > 0 {
        return x
    }
    return nil
}

func clampToPositive(_ x: Int) -> Int {
    guard x > 0 else { return 0 }
    return x
}

var n = 3
while n > 0 { n -= 1 }
repeat { n += 1 } while n < 3

var total = 0
for i in 0 ..< 5 { total += i }

let _ = (grade, firstPositive([-1, 2, -3]), clampToPositive(7), total)
