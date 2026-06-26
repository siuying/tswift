// `@autoclosure` defers the argument expression into a thunk that is only
// evaluated when the parameter is called.
func logIfTrue(_ predicate: @autoclosure () -> Bool, _ label: String) {
    if predicate() {
        print("\(label): yes")
    } else {
        print("\(label): no")
    }
}
logIfTrue(2 > 1, "a")
logIfTrue(1 > 2, "b")

// Laziness: the side-effecting fallback runs only when actually needed.
var calls = 0
func side() -> Int {
    calls += 1
    return 99
}
func orElse(_ value: Int?, _ fallback: @autoclosure () -> Int) -> Int {
    if let v = value { return v }
    return fallback()
}
print(orElse(5, side()))
print(orElse(nil, side()))
print("calls=\(calls)")

// `@autoclosure` on a method parameter.
struct Checker {
    func check(_ cond: @autoclosure () -> Bool, _ name: String) {
        print("\(name): \(cond())")
    }
}
let c = Checker()
c.check(3 == 3, "eq")
c.check(3 == 4, "neq")
