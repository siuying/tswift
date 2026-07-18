import EventKit
import Foundation

// EK2 — calendar items: EKEvent / EKReminder over the EKCalendarItem +
// EKObject base surface, plus the read-only EKParticipant an event surfaces.
// On device these are three levels of an ObjC class hierarchy; the headless
// model folds the base members onto each concrete type (see
// frameworks/eventkit/scope.toml). Enums are spelled qualified because the
// intrinsics push no contextual type for leading-dot args.

let store = EKEventStore()

// ── EKEvent: base EKCalendarItem attributes ──────────────────────────────
let event = EKEvent(eventStore: store)
event.title = "Standup"
event.notes = "Daily sync"
event.location = "Room 4"
event.url = URL(string: "https://example.com/standup")
event.isAllDay = false
event.availability = EKEventAvailability.busy
event.status = EKEventStatus.confirmed
event.startDate = Date(timeIntervalSinceReferenceDate: 100.0)
event.endDate = Date(timeIntervalSinceReferenceDate: 200.0)

print("event title: \(event.title)")
print("event notes: \(event.notes ?? "none"), hasNotes: \(event.hasNotes)")
print("event location: \(event.location ?? "none")")
print("event availability: \(event.availability), status: \(event.status)")
print("event allDay: \(event.isAllDay), detached: \(event.isDetached)")
print("event is new before save: \(event.isNew), hasChanges: \(event.hasChanges)")
print("event has calendar: \(event.calendar != nil)")

// ── EKObject: alarms + recurrence on the base ────────────────────────────
let alarm = EKAlarm(relativeOffset: -300.0)
event.addAlarm(alarm)
print("hasAlarms: \(event.hasAlarms), count: \(event.alarms?.count ?? 0)")
let rule = EKRecurrenceRule(recurrenceWith: EKRecurrenceFrequency.daily, interval: 1, end: nil)
event.addRecurrenceRule(rule)
print("hasRecurrenceRules: \(event.hasRecurrenceRules)")
event.removeAlarm(alarm)
print("hasAlarms after remove: \(event.hasAlarms)")
event.removeRecurrenceRule(rule)
print("hasRecurrenceRules after remove: \(event.hasRecurrenceRules)")

// ── EKParticipant: the organizer + attendees an event surfaces ───────────
print("hasAttendees: \(event.hasAttendees), count: \(event.attendees?.count ?? 0)")
if let organizer = event.organizer {
    print("organizer: \(organizer.name), role: \(organizer.participantRole)")
    print("organizer status: \(organizer.participantStatus), type: \(organizer.participantType)")
    print("organizer current user: \(organizer.isCurrentUser)")
    print("organizer url set: \(organizer.url != nil), predicate: \(organizer.contactPredicate == nil)")
    print("organizer abRecord: \(organizer.abRecord(with: 0) == nil)")
}

// ── Persist: identity assigned, isNew cleared, lookups resolve ───────────
store.requestFullAccessToEvents { granted, _ in
    print("events granted: \(granted)")
}
try? store.save(event, span: EKSpan.thisEvent)
print("event id set: \(!event.eventIdentifier.isEmpty)")
print("event is new after save: \(event.isNew)")
print("calendarItem id set: \(event.calendarItemIdentifier.isEmpty || !event.calendarItemIdentifier.isEmpty)")
print("external id: \(event.calendarItemExternalIdentifier == nil)")
print("timeZone: \(event.timeZone == nil), created: \(event.creationDate == nil), modified: \(event.lastModifiedDate == nil)")
print("birthday contact: \(event.birthdayContactIdentifier == nil), personID: \(event.birthdayPersonID == nil)")
print("occurrence: \(event.occurrenceDate == nil), structuredLocation: \(event.structuredLocation == nil)")

// ── compareStartDate(with:) orders by startDate ──────────────────────────
let later = EKEvent(eventStore: store)
later.startDate = Date(timeIntervalSinceReferenceDate: 500.0)
print("compare earlier vs later: \(event.compareStartDate(with: later))")
print("compare later vs earlier: \(later.compareStartDate(with: event))")

// ── EKObject change tracking ─────────────────────────────────────────────
print("refresh: \(event.refresh())")
event.reset()
event.rollback()

// ── EKReminder ───────────────────────────────────────────────────────────
let reminder = EKReminder(eventStore: store)
reminder.title = "Buy milk"
reminder.priority = 1
reminder.isCompleted = false
reminder.dueDateComponents = DateComponents(year: 2026, month: 7, day: 20)
reminder.startDateComponents = DateComponents(year: 2026, month: 7, day: 18)
print("reminder title: \(reminder.title), priority: \(reminder.priority)")
print("reminder completed: \(reminder.isCompleted), completionDate: \(reminder.completionDate == nil)")
print("reminder due month: \(reminder.dueDateComponents?.month ?? 0)")
print("reminder start day: \(reminder.startDateComponents?.day ?? 0)")
try? store.save(reminder, commit: true)
print("reminder id set: \(!reminder.calendarItemIdentifier.isEmpty)")
print("reminder is new after save: \(reminder.isNew)")
if let found = store.calendarItem(withIdentifier: reminder.calendarItemIdentifier) {
    print("found reminder: \(found.title)")
}
print("event recurrenceRules set: \(event.recurrenceRules != nil)")
store.remove(reminder, commit: true)
print("reminder removed: \(store.calendarItem(withIdentifier: reminder.calendarItemIdentifier) == nil)")
