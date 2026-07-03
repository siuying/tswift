// ClosedRange members — 1...5 always has count 5; 5...1 traps at construction.
let r = 1...5
print(r.lowerBound)
print(r.upperBound)
print(r.count)
print(r.isEmpty)
print(r.startIndex)
print(r.endIndex)
print(r.first!)
print(r.last!)
print(r.min!)
print(r.max!)
print(r.description)
print(r.debugDescription)
print(r.hashValue == (1...5).hashValue)
print(r.hashValue == (1...6).hashValue)
print(r.contains(1))
print(r.contains(5))
print(r.contains(6))
print(r.overlaps(3...8))
print(r.overlaps(6...8))
print(r.clamped(to: 2...4))
print(r.distance(from: 1, to: 4))
print(r.index(1, offsetBy: 2))
// for-in
var sum = 0
for i in 1...5 { sum += i }
print(sum)
// map
let doubled = (1...3).map { $0 * 2 }
print(doubled)
