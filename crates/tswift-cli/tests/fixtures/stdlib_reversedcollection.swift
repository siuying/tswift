// ReversedCollection — lazy reversed view.
let arr = [10, 20, 30, 40, 50]
let rev = arr.reversed()
print(rev.count)
print(rev.isEmpty)
print(rev.first!)
print(rev.last!)
print(rev.startIndex)
print(rev.endIndex)
print(rev.contains(30))
print(rev.contains(99))
// for-in iterates in reversed order
var out: [Int] = []
for x in rev { out.append(x) }
print(out)
// map
let mapped = rev.map { $0 * 2 }
print(mapped)
// reversed() round-trip returns the original base
let base = rev.reversed()
print(base)
// distance and index
print(rev.distance(from: 0, to: 3))
print(rev.index(0, offsetBy: 2))
// hashValue is stable
print(rev.hashValue == arr.reversed().hashValue)
// String reversed iterates characters
let s = "abc"
let sr = s.reversed()
var chars: [String] = []
for c in sr { chars.append(c) }
print(chars)
