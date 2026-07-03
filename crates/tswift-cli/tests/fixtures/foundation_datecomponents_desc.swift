import Foundation

// Case A: timeZone set, no calendar
var dcA = DateComponents()
dcA.timeZone = TimeZone(identifier: "UTC")!
dcA.year = 2024
dcA.month = 6
dcA.day = 21
print(dcA.description)
print(dcA.debugDescription)

// Case B: calendar set (UTC timezone), no explicit timeZone
var cal = Calendar(identifier: .gregorian)
cal.timeZone = TimeZone(identifier: "UTC")!
var dcB = DateComponents()
dcB.calendar = cal
dcB.year = 2024
dcB.month = 6
dcB.day = 21
print(dcB.description)
print(dcB.debugDescription)

// Case C: both calendar and timeZone set
var dcC = DateComponents()
dcC.calendar = cal
dcC.timeZone = TimeZone(identifier: "UTC")!
dcC.year = 2024
dcC.month = 6
print(dcC.description)
print(dcC.debugDescription)

// TimeZone properties
let tz = TimeZone(identifier: "UTC")!
print(tz.identifier)
print(tz.description)

// calendar.timeZone properties
print(cal.timeZone.identifier)
print(cal.timeZone.description)

// calendar.locale?.identifier (empty by default on Gregorian calendar)
print(cal.locale?.identifier ?? "nil")

// DateComponents(calendar:) init stores calendar
let dcD = DateComponents(calendar: cal, year: 2024, month: 3)
print(dcD.calendar != nil)
print(dcD.description)
