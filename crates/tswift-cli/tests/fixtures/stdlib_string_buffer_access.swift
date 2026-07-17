// String / Substring contiguous-buffer access: withUTF8 and withCString.
// The closure receives the code units as a contiguous buffer (read-only tier).

// withUTF8: the UTF-8 code units as a UInt8 buffer.
print("hi".withUTF8 { $0.count })
let sum = "AB".withUTF8 { buf -> Int in
    var total = 0
    for byte in buf { total += Int(byte) }
    return total
}
print(sum)

// withCString: null-terminated CChar buffer (trailing 0 included).
print("hi".withCString { $0.count })
let last = "AB".withCString { buf in Int(buf[buf.count - 1]) }
print(last)

// Both work on a Substring receiver too.
let sub = "hello world".suffix(5)
print(sub.withUTF8 { $0.count })
print(sub.withCString { $0.count })
