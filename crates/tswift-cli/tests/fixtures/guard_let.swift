// `guard let` binds an unwrapped optional for the rest of the scope, or exits.
func greet(_ name: String?) {
    guard let name = name else {
        print("no name")
        return
    }
    print("hi \(name)")
}
greet("Ada")
greet(nil)

// Multiple bindings plus a boolean clause in one guard.
func describe(_ a: Int?, _ b: Int?) -> String {
    guard let a = a, let b = b, a < b else {
        return "invalid"
    }
    return "\(a) < \(b)"
}
print(describe(1, 2))
print(describe(2, 1))
print(describe(nil, 3))

// `guard case` with an enum-case pattern.
enum Reply {
    case ok(Int)
    case fail
}
func code(_ r: Reply) -> Int {
    guard case let .ok(n) = r else {
        return -1
    }
    return n
}
print(code(.ok(200)))
print(code(.fail))
