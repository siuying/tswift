import Foundation

// Headline reference-semantics: `let`-bound DateFormatter accepts property
// writes via the interpreter's set_object_field path (Object, not Struct).

// 2024-06-29 12:34:56 UTC
let date = Date(timeIntervalSince1970: 1719664496.0)

// 1. Basic let-binding + property set + string(from:).
let f = DateFormatter()
f.dateFormat = "yyyy-MM-dd"
print(f.string(from: date))

// 2. Overwrite the property — same Object reflects the new value.
f.dateFormat = "HH:mm:ss"
print(f.string(from: date))

// 3. Alias shares the same Object; mutation through alias is visible via f.
let g = f
g.dateFormat = "MMMM d, yyyy"
print(f.string(from: date))
print(g.string(from: date))
