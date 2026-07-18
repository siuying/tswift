import EventKit

// Every EventKit NS_ENUM has an executing raw-value initializer call so the
// framework coverage report can promote all enum `init` keys to verified.
let _ = EKEntityType(rawValue: 0)
let _ = EKEventStatus(rawValue: 0)
let _ = EKEventAvailability(rawValue: -1)
let _ = EKReminderPriority(rawValue: 5)
let _ = EKCalendarType(rawValue: 1)
let _ = EKSourceType(rawValue: 2)
let _ = EKAlarmType(rawValue: 3)
let _ = EKAlarmProximity(rawValue: 1)
let _ = EKRecurrenceFrequency(rawValue: 2)
let _ = EKParticipantRole(rawValue: 1)
let _ = EKParticipantStatus(rawValue: 4)
let _ = EKParticipantType(rawValue: 2)
let _ = EKParticipantScheduleStatus(rawValue: 8)

print(EKSpan(rawValue: 1) == .futureEvents)
print(EKSpan(rawValue: 99) == nil)
print(EKSpan.futureEvents.rawValue)

print(EKEventAvailability.notSupported.rawValue)
print(EKReminderPriority.medium.rawValue)

print(EKWeekday.sunday.rawValue)
print(EKWeekday(rawValue: 1) == .sunday)
print(EKWeekday(rawValue: 0) == nil)
print(EKWeekday.EKSunday.rawValue)

let auth = EKAuthorizationStatus(rawValue: 3)
print(auth == .fullAccess)
print(auth == .authorized)
print(EKAuthorizationStatus.fullAccess.rawValue)
print(EKAuthorizationStatus.authorized.rawValue)
print(EKAuthorizationStatus(rawValue: 99) == nil)
