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

// Additional stored components.
let full = DateComponents(
    era: 1, year: 2024, month: 6, day: 29, nanosecond: 500,
    weekdayOrdinal: 5, weekOfMonth: 5, yearForWeekOfYear: 2024
)
print(full.era!)
print(full.nanosecond!)
print(full.weekdayOrdinal!)
print(full.weekOfMonth!)
print(full.yearForWeekOfYear!)

// dayOfYear and era resolved through the calendar.
let cal = Calendar(identifier: .gregorian)
let date = cal.date(from: DateComponents(year: 2024, month: 6, day: 29))!
let comps = cal.dateComponents([.dayOfYear, .era], from: date)
print(comps.dayOfYear!)
print(comps.era!)

// Equality now distinguishes the era component.
print(DateComponents(era: 1, year: 2024) == DateComponents(era: 0, year: 2024))
