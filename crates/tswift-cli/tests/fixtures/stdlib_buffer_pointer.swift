// Buffer-pointer access across the contiguous collections and Strings.

// 1. Array.withUnsafeBufferPointer — closure sees the elements as a buffer.
let nums = [1, 2, 3, 4]
let total = nums.withUnsafeBufferPointer { buf -> Int in
    var s = 0
    for x in buf { s += x }
    return s
}
print(total)                  // 10

// 2. Array.withContiguousStorageIfAvailable — arrays are always contiguous,
//    so the closure runs and the result is wrapped in an Optional.
let firstDoubled = nums.withContiguousStorageIfAvailable { buf in
    buf[0] * 2
}
print(firstDoubled ?? -1)     // 2
print(firstDoubled != nil)    // true

// 3. ContiguousArray shares the same buffer access.
let ca: ContiguousArray<Int> = [5, 6, 7]
let caCount = ca.withUnsafeBufferPointer { $0.count }
print(caCount)                // 3
let caSum = ca.withContiguousStorageIfAvailable { buf -> Int in
    buf.reduce(0, +)
}
print(caSum ?? -1)            // 18

// 4. ArraySlice exposes its window as a contiguous buffer.
let slice = nums[1..<3]
let sliceMax = slice.withUnsafeBufferPointer { buf in
    buf.max() ?? 0
}
print(sliceMax)               // 3
let sliceStore = slice.withContiguousStorageIfAvailable { $0.count }
print(sliceStore ?? -1)       // 2

// 5. String / Substring are NOT contiguous over Characters, so
//    withContiguousStorageIfAvailable returns nil without running the closure.
let text = "hello"
let strStore = text.withContiguousStorageIfAvailable { _ in 99 }
print(strStore == nil)        // true
let start = text.startIndex
let mid = text.index(start, offsetBy: 3)
let sub = text[start..<mid]   // "hel" (a Substring)
let subStore = sub.withContiguousStorageIfAvailable { _ in 99 }
print(subStore == nil)        // true
