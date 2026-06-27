// MemoryLayout<T>.size / .stride / .alignment, modelled on a 64-bit platform.
// Primitive scalars and user structs (laid out field-by-field with C-style
// alignment and tail padding) are supported.

print(MemoryLayout<Int>.size)
print(MemoryLayout<Int>.stride)
print(MemoryLayout<Int>.alignment)

print(MemoryLayout<Bool>.size)
print(MemoryLayout<Int8>.size)
print(MemoryLayout<Int16>.stride)
print(MemoryLayout<Int32>.size)
print(MemoryLayout<Float>.size)
print(MemoryLayout<Double>.alignment)

// A struct with a small field followed by a wide one: 1 byte + 7 padding + 8.
struct Pair {
    var flag: Int8
    var value: Int
}
print(MemoryLayout<Pair>.size, MemoryLayout<Pair>.stride, MemoryLayout<Pair>.alignment)

// A struct whose fields pack with no tail padding.
struct Flat {
    var x: Int32
    var y: Int32
}
print(MemoryLayout<Flat>.size, MemoryLayout<Flat>.stride, MemoryLayout<Flat>.alignment)

// Tail padding: a wide field followed by a small one — size 9, stride 16.
struct Tail {
    var value: Int
    var flag: Int8
}
print(MemoryLayout<Tail>.size, MemoryLayout<Tail>.stride, MemoryLayout<Tail>.alignment)

// A nested struct field reuses its tail padding: Tail (size 9) + Int8 = 10,
// rounded to a stride of 16.
struct Nested {
    var inner: Tail
    var extra: Int8
}
print(MemoryLayout<Nested>.size, MemoryLayout<Nested>.stride, MemoryLayout<Nested>.alignment)
