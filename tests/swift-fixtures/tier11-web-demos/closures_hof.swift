// expected-no-diagnostics
// Tier 11 / Web demo — Closures & HOF: map/filter/reduce, chaining, first-class fns.

let numbers = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]

let doubled = numbers.map { $0 * 2 }
print("doubled: \(doubled)")

let evens = numbers.filter { $0 % 2 == 0 }
print("evens: \(evens)")

let sum = numbers.reduce(0, +)
print("sum: \(sum)")

// Chaining
let sumOfSquaredEvens = numbers
    .filter { $0 % 2 == 0 }
    .map    { $0 * $0 }
    .reduce(0, +)
print("sum of squared evens: \(sumOfSquaredEvens)")

// Closure as first-class value
func makeMultiplier(_ factor: Int) -> (Int) -> Int {
    return { $0 * factor }
}
let triple = makeMultiplier(3)
print("triple(7) = \(triple(7))")

let words = ["swift", "is", "fast", "and", "safe"]
let shout = words.map { $0.uppercased() }.joined(separator: " ")
print(shout)
