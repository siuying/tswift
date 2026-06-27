// S10 — slices & remaining containers.
let ca = ContiguousArray([1, 2, 3])
print(ca.count, ca.map { $0 * 2 })
print(ca.reduce(0) { $0 + $1 })

let one = CollectionOfOne(42)
print(Array(one), one.count)

// ArraySlice participates in the shared algorithm layer.
let slice = [1, 2, 3, 4, 5].prefix(3)
print(slice.count, slice.reduce(0) { $0 + $1 }, slice.contains(2))
let tail = [1, 2, 3, 4, 5].dropFirst(2)
print(Array(tail), tail.map { $0 * 10 })

// Result — success/failure, get() throwing, pattern matching.
enum MyError: Error { case bad }
func parse(_ s: String) -> Result<Int, MyError> {
    if let n = Int(s) { return .success(n) }
    return .failure(.bad)
}
let good = parse("42")
print(try! good.get())
let bad = parse("oops")
switch bad {
case .success(let v): print("ok", v)
case .failure: print("failed")
}
