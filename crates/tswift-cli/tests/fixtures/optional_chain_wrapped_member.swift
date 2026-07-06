// Optional chaining `s?.member` reaches the WRAPPED type's member (and the
// whole access is re-wrapped in an optional), while plain `.member` access
// still hits the Optional-owned override.
let s: String? = "hi"
print(s?.debugDescription)
print(s?.count)
var z: String? = "x"
print(z.debugDescription)
let n: String? = nil
print(n?.debugDescription)
