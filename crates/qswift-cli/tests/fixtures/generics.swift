func myMax<T: Comparable>(_ a: T, _ b: T) -> T {
    return a > b ? a : b
}
print(myMax(3, 9))
print(myMax("cat", "dog"))
func sumAll<T>(_ xs: [T], _ start: Int, _ value: (T) -> Int) -> Int {
    return xs.reduce(start) { $0 + value($1) }
}
print(sumAll([1, 2, 3, 4], 0) { $0 })
func pair<A, B>(_ a: A, _ b: B) -> (A, B) { return (a, b) }
let p = pair("x", 10)
print(p.0, p.1)
func firstOf<T>(_ xs: [T]) -> T { return xs[0] }
print(firstOf([7, 8, 9]))
