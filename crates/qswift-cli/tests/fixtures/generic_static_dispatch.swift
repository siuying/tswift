// Static dispatch through a generic type parameter (`T.member`), driven by the
// `Self` requirements of a protocol constraint. The placeholder `T` is bound to
// the concrete argument type at the call.
protocol Addable {
    static func + (lhs: Self, rhs: Self) -> Self
    static func zero() -> Self
}

struct Vec2: Addable {
    var x: Int
    var y: Int
    static func + (lhs: Vec2, rhs: Vec2) -> Vec2 { Vec2(x: lhs.x + rhs.x, y: lhs.y + rhs.y) }
    static func zero() -> Vec2 { Vec2(x: 0, y: 0) }
}

func sumAll<T: Addable>(_ items: [T]) -> T {
    var acc = T.zero()
    for it in items { acc = acc + it }
    return acc
}

let r = sumAll([Vec2(x: 1, y: 2), Vec2(x: 3, y: 4), Vec2(x: 5, y: 6)])
print(r.x, r.y)

// Static stored property through the placeholder.
protocol HasZero { static var origin: Self { get } }
struct Counter: HasZero {
    var n: Int
    static let origin = Counter(n: 0)
}
func start<T: HasZero>(_ example: T) -> T { T.origin }
print(start(Counter(n: 5)).n)
