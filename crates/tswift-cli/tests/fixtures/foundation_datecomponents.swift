import Foundation

let dc = DateComponents(year: 2024, month: 6, day: 29, hour: 9, minute: 41)
print(dc.year!)
print(dc.month!)
print(dc.day!)
print(dc.hour!)
print(dc.minute!)
print(dc.second == nil)
print(dc.isValidDate)

let partial = DateComponents(weekday: 3, weekOfYear: 26)
print(partial.weekday!)
print(partial.weekOfYear!)
print(partial.year == nil)
print(partial.isValidDate)

let invalid = DateComponents(year: 2024, month: 13, day: 5)
print(invalid.isValidDate)

let a = DateComponents(year: 2024, month: 6, day: 29)
let b = DateComponents(year: 2024, month: 6, day: 29)
let c = DateComponents(year: 2024, month: 6, day: 30)
print(a == b)
print(a == c)
print(a != c)
