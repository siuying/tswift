import Foundation

let cal = Calendar(identifier: .gregorian)

// ── description / debugDescription ─────────────────────────────────────────
print(cal.description)
print(cal.debugDescription)

// ── locale / timeZone ──────────────────────────────────────────────────────
print(cal.locale?.identifier ?? "nil")
print(cal.timeZone.identifier)

// ── compare(_:to:toGranularity:) ───────────────────────────────────────────
let d1 = cal.date(from: DateComponents(year: 2024, month: 3, day: 10, hour: 9))!
let d2 = cal.date(from: DateComponents(year: 2024, month: 3, day: 15, hour: 17))!
let d3 = cal.date(from: DateComponents(year: 2024, month: 4, day: 1))!
let d4 = cal.date(from: DateComponents(year: 2025, month: 3, day: 10))!

// same month, different day → .orderedSame at .month granularity
print(cal.compare(d1, to: d2, toGranularity: .month) == .orderedSame)
// different month → .orderedAscending at .month
print(cal.compare(d1, to: d3, toGranularity: .month) == .orderedAscending)
// different day, same month → .orderedAscending at .day
print(cal.compare(d1, to: d2, toGranularity: .day) == .orderedAscending)
// different year → .orderedAscending at .year
print(cal.compare(d1, to: d4, toGranularity: .year) == .orderedAscending)
// reversed: d2 vs d1 at .month → .orderedSame (same month)
print(cal.compare(d2, to: d1, toGranularity: .month) == .orderedSame)

// ── dateInterval(of:for:) ──────────────────────────────────────────────────
let feb15_2024 = cal.date(from: DateComponents(year: 2024, month: 2, day: 15))!

let dayInterval = cal.dateInterval(of: .day, for: feb15_2024)!
print(dayInterval.duration)   // 86400.0
let ds = cal.dateComponents([.year, .month, .day, .hour], from: dayInterval.start)
print(ds.year!)    // 2024
print(ds.month!)   // 2
print(ds.day!)     // 15
print(ds.hour!)    // 0

let monthInterval = cal.dateInterval(of: .month, for: feb15_2024)!
print(monthInterval.duration)  // 2505600.0  (29 × 86400, leap Feb 2024)
let ms = cal.dateComponents([.year, .month, .day], from: monthInterval.start)
print(ms.year!)    // 2024
print(ms.month!)   // 2
print(ms.day!)     // 1

let yearInterval = cal.dateInterval(of: .year, for: feb15_2024)!
print(yearInterval.duration)   // 31622400.0  (366 × 86400, 2024 leap year)
let ys = cal.dateComponents([.year, .month, .day], from: yearInterval.start)
print(ys.year!)    // 2024
print(ys.month!)   // 1
print(ys.day!)     // 1

// ── range(of:in:for:) ──────────────────────────────────────────────────────
let feb10_2024 = cal.date(from: DateComponents(year: 2024, month: 2, day: 10))!
let feb10_2023 = cal.date(from: DateComponents(year: 2023, month: 2, day: 10))!

// Days in February 2024 (leap: 29 days) → 1..<30
let daysLeap = cal.range(of: .day, in: .month, for: feb10_2024)!
print(daysLeap.lowerBound)    // 1
print(daysLeap.upperBound)    // 30

// Days in February 2023 (non-leap: 28 days) → 1..<29
let daysNonLeap = cal.range(of: .day, in: .month, for: feb10_2023)!
print(daysNonLeap.lowerBound) // 1
print(daysNonLeap.upperBound) // 29

// Months in a year → always 1..<13
let monthsInYear = cal.range(of: .month, in: .year, for: feb10_2024)!
print(monthsInYear.lowerBound) // 1
print(monthsInYear.upperBound) // 13

// ── minimumRange(of:) / maximumRange(of:) ─────────────────────────────────
let minDay = cal.minimumRange(of: .day)!
print(minDay.lowerBound)  // 1  (shortest month = Feb non-leap = 28 days)
print(minDay.upperBound)  // 29

let maxDay = cal.maximumRange(of: .day)!
print(maxDay.lowerBound)  // 1  (longest month = 31 days)
print(maxDay.upperBound)  // 32

let minMonth = cal.minimumRange(of: .month)!
print(minMonth.lowerBound) // 1
print(minMonth.upperBound) // 13

// ── ordinality(of:in:for:) ─────────────────────────────────────────────────
let mar1_2024 = cal.date(from: DateComponents(year: 2024, month: 3, day: 1))!
let mar1_2023 = cal.date(from: DateComponents(year: 2023, month: 3, day: 1))!

// Day of year: Mar 1 2024 (leap) = 31 + 29 + 1 = 61
print(cal.ordinality(of: .day, in: .year, for: mar1_2024)!)  // 61
// Day of year: Mar 1 2023 (non-leap) = 31 + 28 + 1 = 60
print(cal.ordinality(of: .day, in: .year, for: mar1_2023)!)  // 60
// Day of month
print(cal.ordinality(of: .day, in: .month, for: mar1_2024)!) // 1
// Month of year
print(cal.ordinality(of: .month, in: .year, for: mar1_2024)!) // 3

// ── nextDate(after:matching:matchingPolicy:) ───────────────────────────────
// 2024-03-10 is a Sunday (weekday=1) at 09:30:00 UTC
let sun0930 = cal.date(from: DateComponents(year: 2024, month: 3, day: 10, hour: 9, minute: 30))!

// Next midnight (hour=0, minute=0) after 09:30 → 2024-03-11 00:00:00 UTC
let nxtMidnight = cal.nextDate(after: sun0930, matching: DateComponents(hour: 0, minute: 0), matchingPolicy: .nextTime)!
let nc = cal.dateComponents([.year, .month, .day, .hour, .minute], from: nxtMidnight)
print(nc.year!)   // 2024
print(nc.month!)  // 3
print(nc.day!)    // 11
print(nc.hour!)   // 0
print(nc.minute!) // 0

// Next Monday (weekday=2) after Sunday 2024-03-10 → 2024-03-11 00:00:00
// Foundation .nextTime semantics: unspecified smaller components (hour/min/sec)
// default to their minimum (0) so the result is midnight of the matching day.
let nxtMonday = cal.nextDate(after: sun0930, matching: DateComponents(weekday: 2), matchingPolicy: .nextTime)!
print(cal.component(.weekday, from: nxtMonday)) // 2
let mc2 = cal.dateComponents([.year, .month, .day, .hour, .minute, .second], from: nxtMonday)
print(mc2.year!)    // 2024
print(mc2.month!)   // 3
print(mc2.day!)     // 11
print(mc2.hour!)    // 0  ← midnight, not 09:30 from `after`
print(mc2.minute!)  // 0
print(mc2.second!)  // 0
