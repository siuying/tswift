// Int reporting-overflow arithmetic.
let r1 = Int.max.addingReportingOverflow(1)
print(r1.partialValue == Int.min, r1.overflow)

let r2 = (10).addingReportingOverflow(5)
print(r2.partialValue, r2.overflow)

let r3 = (3).subtractingReportingOverflow(10)
print(r3.partialValue, r3.overflow)

let r4 = (6).multipliedReportingOverflow(by: 7)
print(r4.partialValue, r4.overflow)

let a: Int8 = 100
let r5 = a.multipliedReportingOverflow(by: 2)
print(r5.overflow)
