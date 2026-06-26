// S3 — ranges & Optional.
let r = 1..<5
print(r.lowerBound, r.upperBound, r.count, r.isEmpty)
print(r.contains(3), r.contains(5), r.contains(0))

let cr = 1...5
print(cr.lowerBound, cr.upperBound, cr.count)
print(cr.contains(5), cr.contains(6))

let empty = 3..<3
print(empty.isEmpty, empty.count)

let clamped = (0..<10).clamped(to: 3..<20)
print(clamped.lowerBound, clamped.upperBound)
let clamped2 = (0...10).clamped(to: 5...8)
print(clamped2.lowerBound, clamped2.upperBound, clamped2.count)

let a: Int? = 5
print(a.map { $0 * 2 } ?? -1)
let b: Int? = nil
print(b.map { $0 * 2 } ?? -1)

let c: Int? = 4
print(c.flatMap { $0 > 0 ? "pos" : nil } ?? "none")
let d: Int? = -1
print(d.flatMap { $0 > 0 ? "pos" : nil } ?? "none")

let count: Int? = 3
print(count.map { $0 + 100 } ?? -1)
