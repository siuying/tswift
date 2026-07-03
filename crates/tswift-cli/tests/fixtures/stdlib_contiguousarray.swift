// Vertical slice 10 — ContiguousArray: semantically identical to Array.

// 1. Constructor from array literal (via ContiguousArray init).
var ca: ContiguousArray<Int> = [10, 20, 30]
print(ca.count)               // 3
print(ca.isEmpty)             // false
print(ca.first!)              // 10
print(ca.last!)               // 30

// 2. startIndex / endIndex.
print(ca.startIndex)          // 0
print(ca.endIndex)            // 3

// 3. Subscript.
print(ca[0])                  // 10
print(ca[2])                  // 30

// 4. Append.
ca.append(40)
print(ca.count)               // 4
print(ca[3])                  // 40

// 5. for-in.
var sum = 0
for x in ca {
    sum += x
}
print(sum)                    // 100

// 6. Array(contiguousArray) conversion.
let arr = Array(ca)
print(arr)                    // [10, 20, 30, 40]

// 7. ContiguousArray(array) constructor.
let arr2 = [1, 2, 3]
var ca2 = ContiguousArray(arr2)
print(ca2.count)              // 3
print(ca2[1])                 // 2

// 8. sort.
var ca3: ContiguousArray<Int> = [5, 3, 1, 4, 2]
ca3.sort()
print(ca3[0])                 // 1
print(ca3[4])                 // 5

// 9. map / filter (sequence algorithms).
let doubled = ca2.map { $0 * 2 }
print(doubled)                // [2, 4, 6]

let evens = ca2.filter { $0 % 2 == 0 }
print(evens)                  // [2]

// 10. Equality.
let ca4: ContiguousArray<Int> = [1, 2, 3]
print(ca2 == ca4)             // true

// 11. description.
print(ca2.description)        // [1, 2, 3]

// 12. hashValue stability.
let h1 = ca2.hashValue
let h2 = ca2.hashValue
print(h1 == h2)               // true

// 13. remove.
ca2.remove(at: 0)
print(ca2.count)              // 2
print(ca2[0])                 // 2

// 14. removeAll.
var ca5: ContiguousArray<Int> = [1, 2, 3, 4]
ca5.removeAll { $0 % 2 == 0 }
print(ca5.count)              // 2
print(ca5[0])                 // 1
print(ca5[1])                 // 3
