import Foundation

let cal = Calendar(identifier: .gregorian)

// Standalone symbol arrays (en_US locale).
print(cal.shortStandaloneMonthSymbols[0])
print(cal.veryShortStandaloneMonthSymbols[0])
print(cal.standaloneWeekdaySymbols[0])
print(cal.shortStandaloneWeekdaySymbols[0])
print(cal.veryShortStandaloneWeekdaySymbols[0])
print(cal.standaloneQuarterSymbols[0])
print(cal.shortStandaloneQuarterSymbols[0])

// Relative-day predicates against a fixed epoch date (1970-01-01),
// which is neither today, tomorrow, nor yesterday.
let epoch = Date(timeIntervalSince1970: 0)
print(cal.isDateInToday(epoch))
print(cal.isDateInTomorrow(epoch))
print(cal.isDateInYesterday(epoch))
