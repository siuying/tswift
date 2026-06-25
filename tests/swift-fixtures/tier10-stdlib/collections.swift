// rust-gap: advanced Tier 0-10 spec syntax not yet modelled by the pure-Rust frontend (tracked in #37)
// expected-no-diagnostics
// Tier 10b — Array / Dictionary / Set with value semantics and higher-order ops.

var numbers = [1, 2, 3]
numbers.append(4)
numbers += [5, 6]

let doubled = numbers.map { $0 * 2 }
let evens = numbers.filter { $0 % 2 == 0 }
let sum = numbers.reduce(0, +)
let descending = numbers.sorted(by: >)

// Value semantics / copy-on-write: mutating the copy leaves the original alone.
var copy = numbers
copy.append(99)

var ages = ["ada": 36, "bob": 40]
ages["carol"] = 28
let adaAge = ages["ada"]
let sortedNames = ages.keys.sorted()

var unique: Set<Int> = [1, 2, 2, 3]
unique.insert(4)
let hasTwo = unique.contains(2)

let paired = Array(zip(numbers, sortedNames))

let _ = (doubled, evens, sum, descending, copy.count, numbers.count, adaAge, sortedNames, unique.count, hasTwo, paired.count)