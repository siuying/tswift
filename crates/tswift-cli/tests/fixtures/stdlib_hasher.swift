// Hasher accumulator + Hashable.hash(into:) across builtin value types.

// combine(_:) / finalize() are deterministic within a run.
var h1 = Hasher()
h1.combine(42)
h1.combine("swift")
h1.combine(true)

var h2 = Hasher()
h2.combine(42)
h2.combine("swift")
h2.combine(true)
print(h1.finalize() == h2.finalize())

// combine order matters.
var h3 = Hasher()
h3.combine("a")
h3.combine("b")
var h4 = Hasher()
h4.combine("b")
h4.combine("a")
print(h3.finalize() == h4.finalize())

// hash(into:) folds a value's digest; equal values hash equally.
func digest<T: Hashable>(_ value: T) -> Int {
    var hasher = Hasher()
    value.hash(into: &hasher)
    return hasher.finalize()
}

let n = 7
var hi = Hasher()
n.hash(into: &hi)
var hj = Hasher()
let m = 7
m.hash(into: &hj)
print(hi.finalize() == hj.finalize())

var hs = Hasher()
"hello".hash(into: &hs)
var ht = Hasher()
"hello".hash(into: &ht)
print(hs.finalize() == ht.finalize())

// Collections, ranges, booleans, doubles, optionals all conform.
var ha = Hasher()
[1, 2, 3].hash(into: &ha)
var hb = Hasher()
[1, 2, 3].hash(into: &hb)
print(ha.finalize() == hb.finalize())

var hr = Hasher()
(0 ..< 5).hash(into: &hr)
var hd = Hasher()
(3.5).hash(into: &hd)
print(hr.finalize() != hd.finalize())

var hset = Hasher()
Set([1, 2, 3]).hash(into: &hset)
var hset2 = Hasher()
Set([3, 2, 1]).hash(into: &hset2)
print(hset.finalize() == hset2.finalize())

let opt: Int? = nil
var ho = Hasher()
opt.hash(into: &ho)
var hbool = Hasher()
false.hash(into: &hbool)
print(ho.finalize() == hbool.finalize())
