// Performance/visibility hint attributes are accepted as no-ops at runtime,
// including on global bindings where the attribute trails the initializer.
@frozen public struct Point {
    public var x = 0
    public var y = 0
}

@inlinable func add(_ a: Int, _ b: Int) -> Int { a + b }

@usableFromInline let base = 5
@usableFromInline var total = 0

@inline(__always) func double(_ n: Int) -> Int { n * 2 }

total = add(base, double(base))
let p = Point(x: 1, y: 2)
print(total)
print(p.x + p.y)
print(add(2, 3))
