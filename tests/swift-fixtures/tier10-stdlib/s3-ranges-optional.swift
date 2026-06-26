// expected-no-diagnostics
// Tier 10a/S3 — ranges & Optional.

let r = 1..<5
let lo = r.lowerBound
let hi = r.upperBound
let n = r.count
let e = r.isEmpty
let has = r.contains(3)

let cr = 1...5
let clamped = (0..<10).clamped(to: 3..<20)

let a: Int? = 5
let mapped = a.map { $0 * 2 }
let b: Int? = nil
let mappedNil = b.map { $0 * 2 }
let c: Int? = 4
let flat = c.flatMap { $0 > 0 ? "pos" : nil }

let _ = (lo, hi, n, e, has, cr.count, clamped.lowerBound, mapped, mappedNil, flat)
