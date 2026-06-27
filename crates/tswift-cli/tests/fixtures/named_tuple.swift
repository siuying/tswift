// Named tuple element access: labeled tuple literals, labeled function return
// types, and the standard-library tuples that carry element labels
// (`quotientAndRemainder`, `enumerated`, `Set.insert`).

func minMax(_ a: [Int]) -> (min: Int, max: Int) {
    return (min: a.min()!, max: a.max()!)
}

let r = minMax([3, 1, 9, 2])
print(r.min, r.max)
print(r.0, r.1)

let point = (x: 10, y: 20)
print(point.x, point.y)
print(point)

let qr = 17.quotientAndRemainder(dividingBy: 5)
print(qr.quotient, qr.remainder)

for pair in ["a", "b", "c"].enumerated() {
    print(pair.offset, pair.element)
}

var seen: Set<Int> = []
let outcome = seen.insert(7)
print(outcome.inserted, outcome.memberAfterInsert)
