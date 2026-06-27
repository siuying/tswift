// Mutating a class instance's collection field through a method writes back to
// the shared storage: array append, dictionary subscript, and a collection
// nested inside a value-type field.
class Store {
    var items: [Int] = []
    var index: [String: Int] = [:]
    func add(_ x: Int) { items.append(x) }
    func tag(_ name: String, _ value: Int) { index[name] = value }
}

let s = Store()
s.add(1)
s.add(2)
s.add(3)
s.tag("a", 10)
s.tag("b", 20)
print(s.items)
print(s.items.count)
print(s.index["a"]! + s.index["b"]!)

struct Bucket { var values: [Int] = [] }
class Holder {
    var bucket = Bucket()
    func push(_ x: Int) { bucket.values.append(x) }
}

let h = Holder()
h.push(7)
h.push(8)
print(h.bucket.values)

// A value-type member copied out keeps value semantics: a later mutation of
// the class field does not affect the earlier snapshot.
let snapshot = h.bucket
h.push(9)
print(snapshot.values)
print(h.bucket.values)

// Aliasing: two references see the same mutation (reference semantics).
let alias = s
alias.add(99)
print(s.items.count)
