// Generic free functions with Comparable / Equatable constraints
// (Generic struct instantiation e.g. Stack<Int>() is not yet supported.)

func largest<T: Comparable>(_ arr: [T]) -> T? {
    if arr.isEmpty { return nil }
    var best = arr[0]
    for x in arr where x > best { best = x }
    return best
}

func allEqual<T: Equatable>(_ values: [T]) -> Bool {
    if values.isEmpty { return true }
    let first = values[0]
    for v in values where v != first { return false }
    return true
}

func swapValues<T>(_ a: inout T, _ b: inout T) {
    let tmp = a; a = b; b = tmp
}

print("largest int: \(largest([3, 1, 4, 1, 5, 9, 2, 6])!)")
print("largest str: \(largest(["banana", "apple", "cherry"])!)")

print("allEqual [1,1,1]: \(allEqual([1, 1, 1]))")
print("allEqual [1,2,1]: \(allEqual([1, 2, 1]))")

var x = 10
var y = 99
swapValues(&x, &y)
print("after swap: x=\(x), y=\(y)")
