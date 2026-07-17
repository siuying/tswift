import Foundation

// == (Equatable)
let a = IndexSet(integersIn: 1..<4)
let b = IndexSet(integersIn: 1..<4)
let c = IndexSet(integer: 9)
print(a == b)
print(a == c)

// description / debugDescription — "\(count) indexes" unconditionally
let s = IndexSet(integersIn: 1..<4)
print(s.description)          // "3 indexes"
print(s.debugDescription)     // "3 indexes"
let one = IndexSet(integer: 7)
print(one.description)        // "1 indexes" (not "1 index" — matches corelibs)
let empty = IndexSet()
print(empty.description)      // "0 indexes"

// intersects(integersIn:)
let is1 = IndexSet(integersIn: 1..<5)
print(is1.intersects(integersIn: 3..<7))
print(is1.intersects(integersIn: 5..<9))

// rangeView (property) — yields maximal contiguous ranges
var mixed = IndexSet()
mixed.insert(1)
mixed.insert(2)
mixed.insert(3)
mixed.insert(7)
for r in mixed.rangeView {
    print(r)
}

// for-in (makeIterator) — iterates integers in ascending order
var iter = IndexSet(integersIn: 10..<13)
let _ = iter.makeIterator()  // exercise the intrinsic
for i in iter {
    print(i)
}

// filteredIndexSet(includeInteger:)
let big = IndexSet(integersIn: 1..<8)
let evens = big.filteredIndexSet { $0 % 2 == 0 }
print(evens.count)
print(evens.contains(2))
print(evens.contains(3))

// shift(startingAt:by:) — positive shift
var sh = IndexSet(integersIn: 1..<5)
sh.shift(startingAt: 3, by: 10)
print(sh.contains(1))
print(sh.contains(2))
print(sh.contains(3))
print(sh.contains(13))
print(sh.contains(14))

// shift with negative delta
var sh2 = IndexSet(integersIn: 5..<9)
sh2.shift(startingAt: 5, by: -3)
print(sh2.contains(2))
print(sh2.contains(3))
print(sh2.contains(4))
print(sh2.contains(5))

// Collection conformance: startIndex/endIndex bound the positions; subscript
// reads the position-th sorted member; index(after:)/index(before:) step a
// position; indexRange(in:) maps an integer range to a position range.
var coll = IndexSet()
coll.insert(2)
coll.insert(5)
coll.insert(9)
print(coll.startIndex)          // 0
print(coll.endIndex)            // 3
print(coll[coll.startIndex])    // 2 (first member)
print(coll[1])                  // 5 (second member)
print(coll[coll.index(after: 1)])   // 9
print(coll[coll.index(before: 2)])  // 5
let ir = coll.indexRange(in: 3..<9)
print(ir.lowerBound)            // 1 (first member >= 3 is 5, at position 1)
print(ir.upperBound)            // 2 (first member >= 9 is 9, at position 2)
