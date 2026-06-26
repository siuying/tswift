// expected-no-diagnostics
// Tier 1c — iterate the non-nil elements of an array of optionals, plus an
// enum-case pattern and a `where` clause.

let values: [Int?] = [1, nil, 3, nil, 5]
var present: [Int] = []
for case let value? in values {
    present.append(value)
}
let _ = present

enum Token {
    case number(Int)
    case symbol(String)
}
let tokens: [Token] = [.number(1), .symbol("+"), .number(2)]
var sum = 0
for case let .number(n) in tokens {
    sum += n
}
let _ = sum

for i in 0..<10 where i % 3 == 0 {
    let _ = i
}
