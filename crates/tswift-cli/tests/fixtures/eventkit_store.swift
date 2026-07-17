import EventKit

// EK1/EK3 — the headless in-memory store: permissions, calendars, sources, and
// non-predicate CRUD. On device EKEventStore talks to the Calendar daemon
// behind a prompt; here it resolves deterministically (see
// frameworks/eventkit/scope.toml). Enums are spelled qualified because the
// store's untyped intrinsics push no contextual type for leading-dot args.

let store = EKEventStore()

// Permissions resolve synchronously — there is no UI to prompt.
store.requestFullAccessToEvents { granted, error in
    print("events granted: \(granted)")
}
if EKEventStore.authorizationStatus(for: EKEntityType.event) == .fullAccess {
    print("events authorization: full")
}

store.requestFullAccessToReminders { granted, _ in
    print("reminders granted: \(granted)")
}
let remindersStatus = EKEventStore.authorizationStatus(for: EKEntityType.reminder)
print("reminders authorization: \(remindersStatus)")

// A fresh store ships one local source and one default calendar.
print("sources: \(store.sources.count)")
print("delegate sources: \(store.delegateSources.count)")
let defaultCalendar = store.defaultCalendarForNewEvents
print("default calendar mutable: \(defaultCalendar?.allowsContentModifications ?? false)")
print("default source: \(store.sources.first?.title ?? "none")")

// Create and persist a new calendar.
let work = EKCalendar(for: EKEntityType.event, eventStore: store)
work.title = "Work"
store.saveCalendar(work, commit: true)
print("calendars after save: \(store.calendars(for: EKEntityType.event).count)")

// Round-trip it through the identifier lookup, then read a source's calendars.
if let found = store.calendar(withIdentifier: work.calendarIdentifier) {
    print("found calendar: \(found.title)")
}
print("source calendars: \(work.source.calendars(for: EKEntityType.event).count)")

// Read the calendar's own attributes and its backing source metadata.
print("calendar type: \(work.type)")
print("calendar immutable: \(work.isImmutable), subscribed: \(work.isSubscribed)")
print("calendar color set: \(work.cgColor != nil)")
print("entity mask: \(work.allowedEntityTypes), availabilities: \(work.supportedEventAvailabilities)")
let source = work.source
print("source: \(source.sourceType), id set: \(!source.sourceIdentifier.isEmpty), delegate: \(source.isDelegate)")

// The write-only and deprecated request spellings resolve too.
store.requestWriteOnlyAccessToEvents { granted, _ in
    print("write-only granted: \(granted)")
}
store.requestAccess(to: EKEntityType.reminder) { granted, _ in
    print("legacy access granted: \(granted)")
}
store.refreshSourcesIfNecessary()
let reminderCalendar = store.defaultCalendarForNewReminders()
print("reminder calendar mutable: \(reminderCalendar?.allowsContentModifications ?? false)")
print("no such calendar item: \(store.calendarItem(withIdentifier: "missing") == nil)")
print("external items: \(store.calendarItems(withExternalIdentifier: "missing").count)")

// Remove the calendar and reset the store.
store.removeCalendar(work, commit: true)
print("calendars after remove: \(store.calendars(for: EKEntityType.event).count)")
store.reset()
store.commit()
print("store id present: \(!store.eventStoreIdentifier.isEmpty)")
