// S8 — String / Character / Substring.
let greeting = "Hello, World"
print(greeting.count, greeting.isEmpty)
print(greeting.first ?? "?", greeting.last ?? "?")
print(greeting.uppercased())
print(greeting.lowercased())
print(greeting.hasPrefix("Hello"), greeting.hasSuffix("World"))
print(greeting.contains("lo, W"), greeting.contains("xyz"))
print(greeting.prefix(5), greeting.suffix(5))

var s = "swift"
s.append("!")
s += "?"
print(s)
print("a" + "b" + "c")

print("a,b,c,d".split(separator: ",").count)
print("one two three".split(separator: " ").map { $0.uppercased() }.joined(separator: "-"))
print(String("racecar".reversed()))

// Extended grapheme clusters: count is by character, not scalar/byte.
print("café".count)
print("e\u{301}".count)
print("🇺🇸".count)
print("👨‍👩‍👧".count)
print("ab🇺🇸c".count)

// Character iteration via the shared sequence layer.
print("hello".filter { $0 != "l" }.count)
