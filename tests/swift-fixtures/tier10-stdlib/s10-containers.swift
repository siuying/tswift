// expected-no-diagnostics
// Tier 10b/S10 — slices & remaining containers.

let ca = ContiguousArray([1, 2, 3])
let mapped = ca.map { $0 * 2 }
let one = CollectionOfOne(42)
let oneArr = Array(one)

let slice = [1, 2, 3, 4, 5].prefix(3)
let sum = slice.reduce(0) { $0 + $1 }
let tail = Array([1, 2, 3, 4, 5].dropFirst(2))

enum MyError: Error { case bad }
func parse(_ s: String) -> Result<Int, MyError> {
    if let n = Int(s) { return .success(n) }
    return .failure(.bad)
}
let good = parse("42")
let value = try! good.get()

let _ = (ca, mapped, oneArr, slice, sum, tail, value)
