import Foundation

// "Weekend" = Saturday + Sunday under this runtime's fixed en_US/Gregorian
// stance (Darwin's real behaviour is Locale-dependent). The calendar is pinned
// to GMT so the output is deterministic regardless of the host time zone.
var cal = Calendar(identifier: .gregorian)
cal.timeZone = TimeZone(identifier: "GMT")!

func show(_ label: String, _ iv: DateInterval?) {
    guard let iv = iv else { print("\(label): nil"); return }
    let c = cal.dateComponents([.year, .month, .day, .hour, .weekday], from: iv.start)
    print("\(label): \(c.year!)-\(c.month!)-\(c.day!) hour=\(c.hour!) weekday=\(c.weekday!) duration=\(iv.duration)")
}

// 2024-06-26 is a Wednesday, 2024-06-29 a Saturday, 2024-06-30 a Sunday.
let wed = cal.date(from: DateComponents(year: 2024, month: 6, day: 26, hour: 12))!
let sat = cal.date(from: DateComponents(year: 2024, month: 6, day: 29, hour: 15))!
let sun = cal.date(from: DateComponents(year: 2024, month: 6, day: 30, hour: 23, minute: 59))!
let mon = cal.date(from: DateComponents(year: 2024, month: 7, day: 1))!

// nextWeekend: first Saturday-midnight strictly after the given date.
show("nextWeekend(after Wed)", cal.nextWeekend(startingAfter: wed))
// From a Saturday, the current weekend already started → next week's.
show("nextWeekend(after Sat)", cal.nextWeekend(startingAfter: sat))

// dateIntervalOfWeekend: the Sat 00:00 ..< Mon 00:00 span containing the date.
show("weekendOf(Sat)", cal.dateIntervalOfWeekend(containing: sat))
show("weekendOf(Sun)", cal.dateIntervalOfWeekend(containing: sun))
// Monday 00:00 is the exclusive end → not part of the weekend.
show("weekendOf(Mon)", cal.dateIntervalOfWeekend(containing: mon))
show("weekendOf(Wed)", cal.dateIntervalOfWeekend(containing: wed))

// autoupdatingCurrent is an alias for current under the fixed-calendar model.
print(Calendar.autoupdatingCurrent.identifier == Calendar.current.identifier)
