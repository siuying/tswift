// Regex literals `/.../` and `#/.../#`, end to end: lexing, parsing, and the
// String matching intrinsics that consume a compiled Regex value.

// A regex literal binds to a reusable value.
let digits = /\d+/
print("abc123def".contains(digits))
print("no digits here".contains(/\d+/))

// firstMatch(of:) yields the whole match at .0 and capture groups after it.
if let m = "order-42".firstMatch(of: /(\w+)-(\d+)/) {
    print(m.0)
    print(m.1)
    print(m.2)
}

// matches(of:) returns every non-overlapping match.
let nums = "a1 b22 c333".matches(of: /\d+/)
print(nums.count)
for n in nums { print(n.0) }

// wholeMatch(of:) only succeeds when the pattern spans the whole string.
print("hello".wholeMatch(of: /[a-z]+/) != nil)
print("hello1".wholeMatch(of: /[a-z]+/) != nil)

// replacing(_:with:) substitutes every match.
print("2024-01-02".replacing(/\d+/, with: "#"))

// Extended `#/.../#` delimiters keep `/` literal.
let path = #/\w+\/\w+/#
print("see src/main here".contains(path))

// Alternation, anchors, and quantifiers.
print("cat".wholeMatch(of: /cat|dog/) != nil)
print("foobar".contains(/^foo/))
