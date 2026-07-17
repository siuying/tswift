import EventKit

// Exhaustive case vocabulary across every in-scope EventKit enum (tiers EK1–EK4,
// see frameworks/eventkit/scope.toml). Each case is spelled fully qualified so
// the builtin-enum resolution is unambiguous; printing the array counts keeps
// the golden deterministic while pinning every case as exercised.

let auth = [
    EKAuthorizationStatus.notDetermined, EKAuthorizationStatus.restricted,
    EKAuthorizationStatus.denied, EKAuthorizationStatus.fullAccess,
    EKAuthorizationStatus.writeOnly, EKAuthorizationStatus.authorized,
]
print("authorization statuses: \(auth.count)")

let entities = [EKEntityType.event, EKEntityType.reminder]
print("entity types: \(entities.count)")

let spans = [EKSpan.thisEvent, EKSpan.futureEvents]
print("spans: \(spans.count)")

let statuses = [
    EKEventStatus.none, EKEventStatus.confirmed, EKEventStatus.tentative,
    EKEventStatus.canceled,
]
print("event statuses: \(statuses.count)")

let avail = [
    EKEventAvailability.notSupported, EKEventAvailability.busy,
    EKEventAvailability.free, EKEventAvailability.tentative,
    EKEventAvailability.unavailable,
]
print("availabilities: \(avail.count)")

let priorities = [
    EKReminderPriority.none, EKReminderPriority.high,
    EKReminderPriority.medium, EKReminderPriority.low,
]
print("priorities: \(priorities.count)")

let calTypes = [
    EKCalendarType.local, EKCalendarType.calDAV, EKCalendarType.exchange,
    EKCalendarType.subscription, EKCalendarType.birthday,
]
print("calendar types: \(calTypes.count)")

let sourceTypes = [
    EKSourceType.local, EKSourceType.exchange, EKSourceType.calDAV,
    EKSourceType.mobileMe, EKSourceType.subscribed, EKSourceType.birthdays,
]
print("source types: \(sourceTypes.count)")

let alarmTypes = [
    EKAlarmType.display, EKAlarmType.audio, EKAlarmType.procedure,
    EKAlarmType.email,
]
print("alarm types: \(alarmTypes.count)")

let proximities = [
    EKAlarmProximity.none, EKAlarmProximity.enter, EKAlarmProximity.leave,
]
print("proximities: \(proximities.count)")

let freqs = [
    EKRecurrenceFrequency.daily, EKRecurrenceFrequency.weekly,
    EKRecurrenceFrequency.monthly, EKRecurrenceFrequency.yearly,
]
print("frequencies: \(freqs.count)")

let roles = [
    EKParticipantRole.unknown, EKParticipantRole.required,
    EKParticipantRole.optional, EKParticipantRole.chair,
    EKParticipantRole.nonParticipant,
]
print("participant roles: \(roles.count)")

let partStatuses = [
    EKParticipantStatus.unknown, EKParticipantStatus.pending,
    EKParticipantStatus.accepted, EKParticipantStatus.declined,
    EKParticipantStatus.tentative, EKParticipantStatus.delegated,
    EKParticipantStatus.completed, EKParticipantStatus.inProcess,
]
print("participant statuses: \(partStatuses.count)")

let partTypes = [
    EKParticipantType.unknown, EKParticipantType.person,
    EKParticipantType.room, EKParticipantType.resource,
    EKParticipantType.group,
]
print("participant types: \(partTypes.count)")

let schedule = [
    EKParticipantScheduleStatus.none, EKParticipantScheduleStatus.pending,
    EKParticipantScheduleStatus.sent, EKParticipantScheduleStatus.delivered,
    EKParticipantScheduleStatus.recipientNotRecognized,
    EKParticipantScheduleStatus.noPrivileges,
    EKParticipantScheduleStatus.deliveryFailed,
    EKParticipantScheduleStatus.cannotDeliver,
    EKParticipantScheduleStatus.recipientNotAllowed,
]
print("schedule statuses: \(schedule.count)")

let weekdays = [
    EKWeekday.sunday, EKWeekday.monday, EKWeekday.tuesday,
    EKWeekday.wednesday, EKWeekday.thursday, EKWeekday.friday,
    EKWeekday.saturday,
]
print("weekdays: \(weekdays.count)")

let legacyWeekdays = [
    EKWeekday.EKSunday, EKWeekday.EKMonday, EKWeekday.EKTuesday,
    EKWeekday.EKWednesday, EKWeekday.EKThursday, EKWeekday.EKFriday,
    EKWeekday.EKSaturday,
]
print("legacy weekdays: \(legacyWeekdays.count)")

// NB: EKType(rawValue:) is deliberately NOT exercised here — builtin enums carry
// no raw values in this runtime, so the RawRepresentable initializer resolves to
// nil rather than the ObjC integer case (see docs/swift-runtime/
// blocked-features.md). The cases above are the faithful surface.
