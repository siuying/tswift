// Generic subscripts: `subscript<T>(...) -> ...`, including on a generic type,
// a writable generic subscript, and a `where` constraint clause. Type
// parameters are erased at runtime; the subscript body runs like any other
// accessor.

struct IntBox {
    var items: [Int]

    // A generic subscript that maps each element through `f`.
    subscript<T>(map f: (Int) -> T) -> [T] {
        return items.map(f)
    }
}

let box = IntBox(items: [1, 2, 3])
print(box[map: { $0 * 2 }])
print(box[map: { "<\($0)>" }])

// Generic subscript on a generic type.
struct Container<Element> {
    var store: [Element]

    subscript<Result>(transform f: (Element) -> Result) -> [Result] {
        return store.map(f)
    }
}

let c = Container(store: [10, 20, 30])
print(c[transform: { $0 + 1 }])
print(c[transform: { "v\($0)" }])

// Writable generic subscript (get + set).
struct Slot {
    var data: [Int]
    subscript<Key>(key k: Key, fallback fb: Int) -> Int {
        get { data.first ?? fb }
        set { data[0] = newValue }
    }
}
var slot = Slot(data: [1, 2, 3])
print(slot["x", 0])
slot["x", 0] = 50
print(slot.data[0])

// Generic subscript with a `where` constraint.
struct Row {
    var values: [Int]
    subscript<T>(describe f: (Int) -> T) -> [String] where T: CustomStringConvertible {
        return values.map { "\(f($0))" }
    }
}
let row = Row(values: [7, 8])
print(row[describe: { $0 * 10 }])
