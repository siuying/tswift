// frontend-gap: chained `.map { "\($0)" }.joined()` on a generic `[T]` trips
// the Rust parser/sema — "expected an expression, found RParen" is reported on
// the following `Stack<Int>()` instantiation because type inference for the
// string-interpolation closure over a free type parameter fails first.
//
// Valid Swift 6. Remove this file once generic map+joined chains type-check.

struct Stack<T> {
    var items: [T] = []
    mutating func push(_ item: T) { items.append(item) }
    // This one-liner form is the gap:
    func display() -> String { items.map { "\($0)" }.joined(separator: " → ") }
}

var s = Stack<Int>()
s.push(1); s.push(2); s.push(3)
print(s.display())
