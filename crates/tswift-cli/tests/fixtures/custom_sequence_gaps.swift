// Custom sequences require declared conformance, prefer makeIterator(), and
// participate in Sequence algorithm dispatch.
struct Both: Sequence, IteratorProtocol {
    var n: Int
    mutating func next() -> Int? {
        if n > 0 { n -= 1; return 99 }
        return nil
    }
    func makeIterator() -> Iter { Iter(n: 3) }
}

struct Iter: IteratorProtocol {
    var n: Int
    mutating func next() -> Int? {
        if n > 0 { let out = n; n -= 1; return out }
        return nil
    }
}

var total = 0
for x in Both(n: 1) { total += x }
print(total)
let doubled = Both(n: 1).map { $0 * 2 }
print(doubled)
