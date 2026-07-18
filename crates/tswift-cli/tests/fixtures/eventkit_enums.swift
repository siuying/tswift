import EventKit

// EventKit's Clang-imported NS_ENUMs, modelled as builtin enums so leading-dot
// spellings resolve against the annotated type and cases switch/compare as on
// device. (The store/value-object surface — EKEventStore, EKEvent, … — lands in
// a later slice; this fixture pins the enum vocabulary across tiers EK1–EK4.)

// EK1 — authorization + span + entity type.
let status = EKAuthorizationStatus.fullAccess
if status == .fullAccess { print("access: full") }

let span: EKSpan = .thisEvent
switch span {
case .thisEvent: print("span: this event")
case .futureEvents: print("span: future events")
}

let entity: EKEntityType = .reminder
switch entity {
case .event: print("entity: event")
case .reminder: print("entity: reminder")
}

// EK2 — calendar-item enums.
let availability: EKEventAvailability = .busy
if availability == .busy { print("availability: busy") }

let priority: EKReminderPriority = .high
switch priority {
case .high: print("priority: high")
default: print("priority: other")
}

// EK3 — calendar & source kinds.
let calType: EKCalendarType = .calDAV
if calType == .calDAV { print("calendar: calDAV") }

// EK4 — alarms, recurrence, participants.
let freq: EKRecurrenceFrequency = .weekly
print("recurrence: \(freq)")

let alarm: EKAlarmType = .display
if alarm == .display { print("alarm: display") }

let day: EKWeekday = .monday
switch day {
case .monday: print("weekday: monday")
default: print("weekday: other")
}

let role: EKParticipantRole = .required
if role == .required { print("participant: required") }
