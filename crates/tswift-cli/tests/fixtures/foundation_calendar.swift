import Foundation

let cal = Calendar(identifier: .gregorian)
print(cal.identifier == .gregorian)

// Round-trip DateComponents -> Date -> DateComponents.
let made = cal.date(from: DateComponents(year: 2024, month: 1, day: 31, hour: 9, minute: 41))!
print(made.timeIntervalSince1970)
let back = cal.dateComponents([.year, .month, .day, .hour, .minute], from: made)
print(back.year!)
print(back.month!)
print(back.day!)
print(back.hour!)
print(back.minute!)

// Single-component queries.
print(cal.component(.weekday, from: made))
print(cal.component(.quarter, from: made))

// Month-boundary arithmetic: Jan 31 + 1 month clamps to Feb 29 (leap year).
let plusMonth = cal.date(byAdding: .month, value: 1, to: made)!
let pm = cal.dateComponents([.year, .month, .day], from: plusMonth)
print(pm.year!)
print(pm.month!)
print(pm.day!)

// Adding a DateComponents delta.
let plusYearDay = cal.date(byAdding: DateComponents(year: 1, day: 5), to: made)!
let pyd = cal.dateComponents([.year, .month, .day], from: plusYearDay)
print(pyd.year!)
print(pyd.month!)
print(pyd.day!)

// startOfDay drops the time-of-day.
let start = cal.startOfDay(for: made)
let s = cal.dateComponents([.hour, .minute, .day], from: start)
print(s.hour!)
print(s.minute!)
print(s.day!)

// Same-day comparison.
print(cal.isDate(made, inSameDayAs: start))
let nextDay = cal.date(byAdding: .day, value: 1, to: made)!
print(cal.isDate(made, inSameDayAs: nextDay))

// Symbol tables (en_US Gregorian).
print(cal.monthSymbols[0])
print(cal.monthSymbols.count)
print(cal.shortMonthSymbols[2])
print(cal.veryShortMonthSymbols[4])
print(cal.weekdaySymbols[0])
print(cal.shortWeekdaySymbols[6])
print(cal.veryShortWeekdaySymbols[1])
print(cal.standaloneMonthSymbols[11])
print(cal.quarterSymbols[0])
print(cal.shortQuarterSymbols[3])
print(cal.eraSymbols[1])
print(cal.longEraSymbols[0])
print(cal.amSymbol)
print(cal.pmSymbol)
print(cal.firstWeekday)
print(cal.minimumDaysInFirstWeek)

// Weekend detection (deterministic).
let saturday = cal.date(from: DateComponents(year: 2024, month: 6, day: 29))!
let sunday = cal.date(from: DateComponents(year: 2024, month: 6, day: 30))!
print(cal.isDateInWeekend(saturday))
print(cal.isDateInWeekend(sunday))
print(cal.isDateInWeekend(made))
