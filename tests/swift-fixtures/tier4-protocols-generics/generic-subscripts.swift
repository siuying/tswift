// Generic subscripts: a `<T>` clause on a `subscript`, on plain and generic
// types, with a `where` constraint. The frontend must parse and type-check all
// of these.
// expected-no-diagnostics

struct IntBox {
    var items: [Int]
    subscript<T>(map f: (Int) -> T) -> [T] {
        return items.map(f)
    }
}

struct Container<Element> {
    var store: [Element]
    subscript<Result>(transform f: (Element) -> Result) -> [Result] {
        return store.map(f)
    }
}

struct Row {
    var values: [Int]
    subscript<T>(describe f: (Int) -> T) -> [String] where T: CustomStringConvertible {
        return values.map { "\(f($0))" }
    }
}
