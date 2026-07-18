// Generic data shaping used by SwiftUI lists: the filtered rows feed ForEach.

func rowLabels<S: Sequence>(_ items: S) -> [String] {
    items.filter { $0 > 1 }.map { "row-\($0)" }
}

let rows = rowLabels([0, 1, 2, 3])
let forEachData = rows
print(forEachData.joined(separator: ","))

struct Steps: Sequence, Collection, IteratorProtocol {
    var n: Int

    mutating func next() -> Int? {
        if n == 0 { return nil }
        let value = n
        n -= 1
        return value
    }

    func makeIterator() -> Steps { self }
}

let steps = Steps(n: 3)
print(steps.map { $0 * 2 })
print(steps.count, steps.isEmpty, steps.first ?? -1, steps.last ?? -1)
print(Array(steps.indices))
print(Array(zip(steps, [10, 20])))
print(steps.reduce(0) { $0 + $1 })
print(steps.reduce(into: [Int]()) { $0.append($1 * 10) })
print([[1, 2], 3...4].flatMap { $0 })
print([[1, 2], [3, 4]].joined(separator: [0]))
print([1, 0, 2, 0, 3].split(maxSplits: 1, whereSeparator: { $0 == 0 }))
