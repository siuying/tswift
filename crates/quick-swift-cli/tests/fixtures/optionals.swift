var maybe: Int? = nil
print(maybe ?? -1)
if let v = maybe { print(v) } else { print("nil") }
maybe = 42
print(maybe!)
let name: String? = "Sam"
print(name?.count ?? 0)
func parse(_ s: Int?) -> Int {
    guard let n = s else { return -1 }
    return n * 2
}
print(parse(10), parse(nil))
let opt: Int? = 9
switch opt {
case .some(let n): print("some \(n)")
case .none: print("none")
}
