import Foundation

// Fixed epoch inputs for deterministic output. All formatting is en_US, UTC.

// 2024-06-21 15:30:45 UTC (a Friday, day-of-year 173, Q2)
let d1 = Date(timeIntervalSince1970: 1718983845.0)
// 2024-01-05 08:07:09 UTC (a Friday, single-digit day, day-of-year 5)
let d2 = Date(timeIntervalSince1970: 1704442029.0)

// --- new component tokens (default widths) ---
print(d1.formatted(.dateTime.weekday()))
print(d1.formatted(.dateTime.era().year()))
print(d1.formatted(.dateTime.quarter().year()))
print(d1.formatted(.dateTime.dayOfYear()))
print(d1.formatted(.dateTime.weekday().month().day()))

// --- month width symbols ---
print(d1.formatted(.dateTime.month(.wide).day().year()))
print(d1.formatted(.dateTime.month(.narrow).day(.twoDigits)))
print(d2.formatted(.dateTime.month(.twoDigits).day(.twoDigits)))

// --- weekday width symbols ---
print(d1.formatted(.dateTime.weekday(.wide)))
print(d1.formatted(.dateTime.weekday(.short)))
print(d1.formatted(.dateTime.weekday(.narrow)))

// --- year width symbols ---
print(d1.formatted(.dateTime.year(.twoDigits)))
print(d1.formatted(.dateTime.year(.padded(6))))

// --- quarter / era widths ---
print(d1.formatted(.dateTime.quarter(.wide)))
print(d1.formatted(.dateTime.era(.wide).year()))

// --- defaultDigits / oneDigit / threeDigits widths ---
print(d1.formatted(.dateTime.month(.defaultDigits).day(.defaultDigits)))
print(d1.formatted(.dateTime.quarter(.oneDigit)))
print(d2.formatted(.dateTime.dayOfYear(.threeDigits)))

// --- combined date + time still joins with " at " ---
print(d1.formatted(.dateTime.weekday().month(.wide).day().hour().minute()))
