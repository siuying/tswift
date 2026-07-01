// expected-no-diagnostics
// oracle-gap: C msf predates SE-0393 parameter packs

func tupleSize<each T>(_ value: repeat each T) -> Int {
    var count = 0
    for _ in repeat each value {
        count += 1
    }
    return count
}

func forwarded<each T>(_ items: repeat each T) -> Int {
    return tupleSize(repeat each items)
}

let _ = tupleSize(1, "two", 3.0)
let _ = forwarded("a", "b")
