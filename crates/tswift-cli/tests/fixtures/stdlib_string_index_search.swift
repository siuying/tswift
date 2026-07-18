// String / Substring index search and index-bound slicing.
// Every position is an extended grapheme-cluster coordinate.
let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}"
let accent = "e\u{301}"
let text = "a\(family)\(accent)b\(family)"

let firstFamily = text.firstIndex(of: family)!
let lastFamily = text.lastIndex(of: family)!
print(text.distance(from: text.startIndex, to: firstFamily))
print(text.distance(from: text.startIndex, to: lastFamily))

let firstB = text.firstIndex(where: { $0 == "b" })!
print(text.distance(from: text.startIndex, to: firstB))

let match = text.range(of: "\(accent)b")!
print(text[match])
print(text.range(of: "missing") == nil)
print(text.prefix(upTo: match.lowerBound))
print(text.suffix(from: match.lowerBound))

// The view retains the original base coordinates: its start is 1, not 0.
let slice = text[firstFamily..<lastFamily]
let accentIndex = slice.firstIndex(of: accent)!
print(slice.distance(from: slice.startIndex, to: accentIndex))
print(slice[slice.range(of: accent)!])
print(slice.prefix(upTo: accentIndex))
print(slice.suffix(from: accentIndex))
let sliceB = slice.firstIndex(where: { $0 == "b" })!
print(slice.distance(from: slice.startIndex, to: sliceB))

var removed = text
removed.removeSubrange(removed.range(of: accent)!)
print(removed)
