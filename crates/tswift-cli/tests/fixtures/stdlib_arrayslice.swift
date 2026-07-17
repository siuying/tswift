// Vertical slice 10 — ArraySlice: view semantics and base-relative indices.

var a = [0, 1, 2, 3, 4, 5]

// 1. Slicing produces an ArraySlice with base-relative startIndex/endIndex.
let sl = a[2..<5]
print(sl.startIndex)          // 2
print(sl.endIndex)            // 5
print(sl.count)               // 3
print(sl.isEmpty)             // false

// 2. Subscript uses base coordinates.
print(sl[2])                  // 2
print(sl[3])                  // 3
print(sl[4])                  // 4

// 3. first / last.
print(sl.first!)              // 2
print(sl.last!)               // 4

// 4. for-in iterates slice elements in order.
var sum = 0
for x in sl {
    sum += x
}
print(sum)                    // 9  (2+3+4)

// 5. Array(slice) materializes.
let arr = Array(sl)
print(arr)                    // [2, 3, 4]
print(arr[0])                 // 2

// 6. Equality.
let sl2 = a[2..<5]
print(sl == sl2)              // true
let sl3 = a[0..<3]
print(sl == sl3)              // false

// 7. Mutation detaches (append).
var slm = a[1..<4]            // [1, 2, 3]
slm.append(99)
print(slm.startIndex)         // 0  (detached)
print(slm.count)              // 4
print(slm[0])                 // 1
print(slm[3])                 // 99
// Original unaffected.
print(a[1])                   // 1
print(a.count)                // 6

// 8. Mutation detaches (remove).
var slr = a[0..<4]            // [0, 1, 2, 3]
let removed = slr.remove(at: 1)
print(removed)                // 1
print(slr.startIndex)         // 0
print(slr.count)              // 3
print(slr[0])                 // 0
print(slr[1])                 // 2

// 9. Chained slicing (sub-slice of slice keeps base coords).
let outer = a[1..<5]          // view into a: indices 1,2,3,4
let inner = outer[2..<4]      // sub-slice: indices 2,3
print(inner.startIndex)       // 2
print(inner.endIndex)         // 4
print(inner[2])               // 2
print(inner[3])               // 3

// 10. sorted (non-mutating via sequence algo).
var unsorted = [5, 3, 1, 4, 2]
let sub2 = unsorted[0..<5]
let s = sub2.sorted()
print(s)                      // [1, 2, 3, 4, 5]

// 11. map / filter via sequence protocol.
let mapped = sl.map { $0 * 2 }
print(mapped)                 // [4, 6, 8]

let filtered = sl.filter { $0 > 2 }
print(filtered)               // [3, 4]

// 12. contains.
print(sl.contains(3))         // true
print(sl.contains(9))         // false

// 13. description / debugDescription.
print(sl.description)         // [2, 3, 4]

// 14. distance/index with base-relative bounds on a non-zero-start slice.
let base = [10, 20, 30, 40, 50]
let nz = base[2..<5]          // startIndex=2, endIndex=5
print(nz.startIndex)          // 2
print(nz.endIndex)            // 5
print(nz.distance(from: 2, to: 5))   // 3
print(nz.index(2, offsetBy: 2))      // 4

// 15. insert at startIndex of a non-zero-start slice translates to local 0.
var nzi = base[2..<5]         // [30, 40, 50]
nzi.insert(99, at: nzi.startIndex)   // at base index 2 → local 0
print(nzi.count)              // 4
print(nzi[0])                 // 99  (detached, 0-based after mutation)
print(nzi[1])                 // 30

// 16. insert(contentsOf:at:) at endIndex of non-zero-start slice.
var nzi2 = base[1..<3]        // [20, 30]
nzi2.insert(contentsOf: [77, 88], at: nzi2.endIndex)  // at base index 3
print(nzi2.count)             // 4
print(nzi2[2])                // 77
print(nzi2[3])                // 88

// 17. ArraySlice(_:) initializer — full-range slice over an Array.
let whole = ArraySlice([7, 8, 9])
print(whole.count)            // 3
print(whole[0])               // 7
print(whole[2])               // 9
