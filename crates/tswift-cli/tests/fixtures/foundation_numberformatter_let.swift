import Foundation

// Headline reference-semantics: `let`-bound NumberFormatter accepts property
// writes via the interpreter's set_object_field path (Object, not Struct).

// 1. Basic let-binding + property set + string(from:).
let nf = NumberFormatter()
nf.numberStyle = .decimal
print(nf.string(from: 1234567)!)
print(nf.string(from: 1234.5)!)

// 2. Overwrite the property — same Object reflects the new value.
nf.numberStyle = .currency
print(nf.string(from: 9.99)!)

// 3. Alias shares the same Object; mutation through alias visible via nf.
let copy = nf
copy.numberStyle = .percent
print(nf.string(from: 0.5)!)
print(copy.string(from: 0.5)!)
