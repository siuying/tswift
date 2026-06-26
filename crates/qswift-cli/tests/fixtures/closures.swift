let nums = [1, 2, 3, 4, 5]
print(nums.map { $0 * $0 })
print(nums.filter { $0 % 2 == 1 })
print(nums.reduce(0) { $0 + $1 })
let makeAdder: (Int) -> ((Int) -> Int) = { x in { y in x + y } }
let add10 = makeAdder(10)
print(add10(5))
let captured = { [base = 100] (n: Int) -> Int in base + n }
print(captured(7))
