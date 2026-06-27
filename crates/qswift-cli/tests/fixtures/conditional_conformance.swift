// Conditional conformance: `Pair` is `Equatable` only when its element is.
struct Pair<T> { var a: T; var b: T }

extension Pair: Equatable where T: Equatable {
    static func == (l: Pair<T>, r: Pair<T>) -> Bool { l.a == r.a && l.b == r.b }
}

let p = Pair(a: 1, b: 2)
let q = Pair(a: 1, b: 2)
let r = Pair(a: 1, b: 9)
print(p == q)
print(p == r)
print(p != r)

// The conditional conformance lets `Pair<Int>` flow through an `Equatable`
// generic constraint.
func allSame<E: Equatable>(_ xs: [E]) -> Bool {
    for x in xs where x != xs[0] { return false }
    return true
}
print(allSame([Pair(a: 1, b: 2), Pair(a: 1, b: 2)]))
print(allSame([Pair(a: 1, b: 2), Pair(a: 3, b: 4)]))
