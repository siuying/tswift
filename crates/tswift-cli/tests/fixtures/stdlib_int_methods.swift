// Int reporting-overflow division, stride and textual members.
let d = (17).dividedReportingOverflow(by: 5)
print(d.partialValue, d.overflow)

let dz = (1).dividedReportingOverflow(by: 0)
print(dz.partialValue, dz.overflow)

let rr = (17).remainderReportingOverflow(dividingBy: 5)
print(rr.partialValue, rr.overflow)

let dmin = Int.min.dividedReportingOverflow(by: -1)
print(dmin.overflow)

print((10).distance(to: 25), (10).advanced(by: 7))
print((42).description)
print((42).hashValue == (42).hashValue, (42).hashValue == (43).hashValue)
