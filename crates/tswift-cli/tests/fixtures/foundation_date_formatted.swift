import Foundation

// Fixed epoch inputs for deterministic output.
// All formatting is en_US, UTC.

// 2024-06-21 15:30:45 UTC
let d1 = Date(timeIntervalSince1970: 1718983845.0)
// 2000-02-29 00:00:00 UTC (leap day)
let leap = Date(timeIntervalSince1970: 951782400.0)
// 2024-01-01 00:00:00 UTC (midnight)
let midnight = Date(timeIntervalSince1970: 1704067200.0)
// 2024-01-01 12:00:00 UTC (noon)
let noon = Date(timeIntervalSince1970: 1704110400.0)

// --- default (abbreviated date + shortened time) ---
print(d1.formatted())

// --- .iso8601 ---
print(d1.formatted(.iso8601))

// --- date:time: combinations ---
print(d1.formatted(date: .numeric, time: .omitted))
print(d1.formatted(date: .abbreviated, time: .shortened))
print(d1.formatted(date: .long, time: .standard))
print(d1.formatted(date: .complete, time: .complete))
print(d1.formatted(date: .omitted, time: .shortened))
print(d1.formatted(date: .omitted, time: .standard))

// --- component chain ---
print(d1.formatted(.dateTime.year().month().day()))
print(d1.formatted(.dateTime.hour().minute().second()))
print(d1.formatted(.dateTime.year().month().day().hour().minute().second()))

// --- edge cases ---
// leap day numeric
print(leap.formatted(date: .numeric, time: .omitted))
// leap day abbreviated
print(leap.formatted(date: .abbreviated, time: .omitted))
// midnight AM
print(midnight.formatted(date: .omitted, time: .shortened))
// noon PM
print(noon.formatted(date: .omitted, time: .shortened))
// complete time style (has GMT suffix)
print(midnight.formatted(date: .omitted, time: .complete))

// --- FormatStyle.format(_:) — the FormatStyle protocol method ---
let style = .dateTime.year().month().day()
print(style.format(d1))
