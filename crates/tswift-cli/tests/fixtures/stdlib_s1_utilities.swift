// S1 — free utilities & output.
print(max(3, 7), min(3, 7))
print(max(2, 9, 4), min(8, 1, 5))
print(abs(-5), abs(7), abs(-2.5))

var a = 10
var b = 20
swap(&a, &b)
print(a, b)

for x in stride(from: 0, to: 10, by: 3) {
    print(x, terminator: " ")
}
print("")
print(Array(stride(from: 1, through: 5, by: 2)))
print(Array(stride(from: 5, to: 0, by: -2)))

print(Array(zip([1, 2, 3], [10, 20, 30])))
print(Array(repeatElement(9, count: 3)))

let powers = sequence(first: 1) { $0 < 16 ? $0 * 2 : nil }
print(Array(powers))

print("a", "b", "c", separator: "-")
debugPrint("quoted", 42)
dump(7)
dump([1, 2, 3])

assert(2 + 2 == 4)
precondition(true)
print("survived assertions")

final class Box { var n = 0 }
var box = Box()
print(isKnownUniquelyReferenced(&box))
let other = box
print(isKnownUniquelyReferenced(&box))
print(other.n)
