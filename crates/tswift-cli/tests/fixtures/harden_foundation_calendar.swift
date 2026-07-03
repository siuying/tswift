import Foundation

// Harden slice 25: Calendar edge cases
// Ground-truth captured from Swift 6.3.2 on macOS.

var cal = Calendar(identifier: .gregorian)
cal.timeZone = TimeZone(identifier: "UTC")!

let ref = Date(timeIntervalSinceReferenceDate: 0)  // 2001-01-01 00:00:00 UTC

// --- Component extraction from reference date ---
print(cal.component(.year, from: ref))     // 2001
print(cal.component(.month, from: ref))    // 1
print(cal.component(.day, from: ref))      // 1
print(cal.component(.hour, from: ref))     // 0
print(cal.component(.minute, from: ref))   // 0
print(cal.component(.weekday, from: ref))  // 2 (Monday in Gregorian with Sunday=1)

// --- Leap year: 2000 is a leap year ---
let leapComp = DateComponents(calendar: cal, year: 2000, month: 2, day: 29)
let leapDate = cal.date(from: leapComp)!
print(cal.component(.day, from: leapDate))    // 29
print(cal.component(.month, from: leapDate))  // 2
print(cal.component(.year, from: leapDate))   // 2000

// --- range of days in February ---
let feb2000 = cal.range(of: .day, in: .month, for: leapDate)!
print(feb2000.count)   // 29 (leap year)

let feb2001Comp = DateComponents(calendar: cal, year: 2001, month: 2, day: 1)
let feb2001 = cal.date(from: feb2001Comp)!
let feb2001Range = cal.range(of: .day, in: .month, for: feb2001)!
print(feb2001Range.count)  // 28 (non-leap)

// --- range of months in year ---
let monthsRange = cal.range(of: .month, in: .year, for: ref)!
print(monthsRange.count)  // 12

// --- ordinality ---
// Feb 29 is the 60th day of a leap year
print(cal.ordinality(of: .day, in: .year, for: leapDate)!)  // 60
// Jan 1 is day 1
print(cal.ordinality(of: .day, in: .year, for: ref)!)        // 1

// --- weekOfYear: mid-year dates where naïve and Foundation agree ---
let jan15Comp = DateComponents(calendar: cal, year: 2024, month: 1, day: 15)
let jan15 = cal.date(from: jan15Comp)!
print(cal.component(.weekOfYear, from: jan15))   // 3

let jul4Comp = DateComponents(calendar: cal, year: 2024, month: 7, day: 4)
let jul4 = cal.date(from: jul4Comp)!
print(cal.component(.weekOfYear, from: jul4))    // 27

// --- compare at granularity ---
let dc1 = DateComponents(calendar: cal, year: 2024, month: 3, day: 15, hour: 9)
let dc2 = DateComponents(calendar: cal, year: 2024, month: 3, day: 15, hour: 17)
let date1 = cal.date(from: dc1)!
let date2 = cal.date(from: dc2)!
// Same day, different hours
print(cal.compare(date1, to: date2, toGranularity: .day) == .orderedSame)   // true
print(cal.compare(date1, to: date2, toGranularity: .hour) == .orderedSame)  // false

// --- isDate(inSameDayAs:) ---
print(cal.isDate(date1, inSameDayAs: date2))          // true (same day)
print(cal.isDateInToday(Date.distantPast))             // false

// --- dateInterval ---
let di = cal.dateInterval(of: .month, for: leapDate)!
// The interval for Feb 2000 spans 29 days = 2505600 seconds
print(di.duration)   // 2505600.0
