// Tier 10b — `isKnownUniquelyReferenced(_:)` drives copy-on-write correctness.
// A unique class instance reads `true`; once shared it reads `false`, and a
// CoW wrapper uses it to copy its backing storage only when shared.

final class Storage {
    var values: [Int]
    init(_ values: [Int]) { self.values = values }
}

struct CowBuffer {
    private var storage: Storage
    init(_ values: [Int]) { storage = Storage(values) }

    var values: [Int] { storage.values }

    mutating func append(_ value: Int) {
        if !isKnownUniquelyReferenced(&storage) {
            print("copying storage")
            storage = Storage(storage.values)
        } else {
            print("mutating in place")
        }
        storage.values.append(value)
    }
}

var unique = Storage([1, 2])
print(isKnownUniquelyReferenced(&unique))
let shared = unique
print(isKnownUniquelyReferenced(&unique))
print(shared.values)

var a = CowBuffer([1, 2, 3])
a.append(4)
var b = a
b.append(5)
print(a.values)
print(b.values)
