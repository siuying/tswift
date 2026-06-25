// oracle-gap: the C msf does not resolve a protocol's associated type
// (`Collection.Element`) inside an extension on the protocol.
// Tier 4c — extensions adding members/inits/subscripts, conditional conformance,
// extensions on generic types.

struct Celsius {
    var degrees: Double
}

extension Celsius {
    var fahrenheit: Double { degrees * 9 / 5 + 32 }
    func roundedDegrees() -> Int { Int(degrees.rounded()) }
    init(fahrenheit: Double) { self.degrees = (fahrenheit - 32) * 5 / 9 }
    subscript(offset: Double) -> Double { degrees + offset }
}

protocol Summable {
    func total() -> Int
}

extension Array: Summable where Element == Int {
    func total() -> Int { reduce(0, +) }
}

extension Collection {
    var secondElement: Element? {
        guard count >= 2 else { return nil }
        return self[index(after: startIndex)]
    }
}

let boiling = Celsius(fahrenheit: 212)
let warmth = boiling.fahrenheit
let whole = boiling.roundedDegrees()
let shifted = boiling[10]
let summed = [1, 2, 3].total()
let second = [10, 20, 30].secondElement

let _ = (warmth, whole, shifted, summed, second)
