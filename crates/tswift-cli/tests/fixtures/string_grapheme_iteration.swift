// String iteration, indexing, Array(), and map all agree with count on
// extended grapheme clusters (Swift Characters), not Unicode scalars.

let s = "cafe\u{301}" // "café" with a combining acute accent
print(s.count)
for c in s { print(c) }
print(Array(s).count)
print(s.map { $0 }.count)
print(s.filter { $0 != "f" }.count)

let flag = "🇺🇸"
print(flag.count)
