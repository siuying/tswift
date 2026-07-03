import Foundation

// --- description / debugDescription ---
// (no calendar/timezone: just int fields, trailing space)
let dc = DateComponents(year: 2024, month: 6, day: 21)
print(dc.description)
print(dc.debugDescription)
print(dc.description == dc.debugDescription)

// full int fields
let full = DateComponents(
    era: 1, year: 2024, month: 6, day: 21,
    hour: 9, minute: 41, second: 30, nanosecond: 500
)
print(full.description)

// empty DateComponents → empty string
let empty = DateComponents()
print(empty.description == "")

// isLeapMonth in description
var dc2 = DateComponents()
dc2.year = 2024
dc2.isLeapMonth = true
print(dc2.description)

var dc2b = DateComponents()
dc2b.isLeapMonth = false
print(dc2b.description)

// --- isLeapMonth property ---
print(empty.isLeapMonth == nil)
print(dc2.isLeapMonth!)

// --- calendar property ---
print(empty.calendar == nil)

var dc5 = DateComponents()
dc5.calendar = Calendar(identifier: .gregorian)
print(dc5.calendar != nil)

// --- timeZone property ---
print(empty.timeZone == nil)

// --- date property ---
// Without calendar → nil
let noCalDC = DateComponents(year: 2024, month: 6, day: 21)
print(noCalDC.date == nil)

// With calendar → resolved date (Calendar.date(from:) semantics)
var cal = Calendar(identifier: .gregorian)
var dc4 = DateComponents()
dc4.calendar = cal
dc4.year = 2024
dc4.month = 6
dc4.day = 21
print(dc4.date!.description)

// Insufficient components still resolve (missing fields default to 1/0)
var dc6 = DateComponents()
dc6.calendar = cal
dc6.hour = 9
// year/month/day all default to 1 → 0001-01-01 09:00:00 UTC
print(dc6.date!.description)

// --- hashValue consistency ---
let a = DateComponents(year: 2024, month: 6, day: 21)
let b = DateComponents(year: 2024, month: 6, day: 21)
let c = DateComponents(year: 2024, month: 6, day: 22)
print(a.hashValue == b.hashValue)
print(a.hashValue == c.hashValue)
