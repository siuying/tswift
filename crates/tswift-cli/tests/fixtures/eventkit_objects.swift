import EventKit

// EventKit's EK4 value objects (EKAlarm, EKStructuredLocation, EKRecurrenceEnd,
// EKRecurrenceDayOfWeek, EKRecurrenceRule) are reference types with mutable
// stored properties. This fixture constructs each, mutates properties in place,
// and reads them back — pinning the settable-property model for framework
// reference types (leading-dot enum spellings use the qualified form since the
// generic builtin constructors are untyped).

// EKAlarm — relative-offset + proximity + attached structured location.
let alarm = EKAlarm(relativeOffset: -300.0)
print("alarm offset: \(alarm.relativeOffset)")
alarm.proximity = EKAlarmProximity.enter
print("alarm proximity: \(alarm.proximity)")

// EKStructuredLocation — title + radius, attached to the alarm.
let location = EKStructuredLocation(title: "Office")
location.radius = 50.0
print("location: \(location.title) r=\(location.radius)")
alarm.structuredLocation = location
print("alarm has location: \(alarm.structuredLocation != nil)")

// EKRecurrenceEnd — count-based recurrence terminator.
let end = EKRecurrenceEnd(occurrenceCount: 10)
print("recurrence ends after: \(end.occurrenceCount)")

// EKRecurrenceDayOfWeek — weekday + ordinal week number.
let dayOfWeek = EKRecurrenceDayOfWeek(dayOfTheWeek: EKWeekday.tuesday, weekNumber: 2)
print("day of week: \(dayOfWeek.dayOfTheWeek) week \(dayOfWeek.weekNumber)")

// EKRecurrenceRule — frequency + interval + end.
let rule = EKRecurrenceRule(recurrenceWith: EKRecurrenceFrequency.weekly, interval: 2, end: end)
print("rule freq: \(rule.frequency) interval: \(rule.interval)")
print("rule end count: \(rule.recurrenceEnd?.occurrenceCount ?? 0)")
