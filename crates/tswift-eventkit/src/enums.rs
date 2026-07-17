//! In-scope EventKit enumerations, modelled as builtin enums.
//!
//! EventKit's enums are Clang-imported `NS_ENUM`s. We model the **cases** and
//! `init(rawValue:)` — the synthesized `Equatable`/`Hashable`/AttributedString-
//! attachment conformance members the importer stamps on every one
//! (`!=`/`hash`/`hashValue`/`makeFromRawAttachmentValue`/
//! `rawAttachmentValueRepresentation`) are conformance boilerplate, not
//! EventKit API, and are scoped out per-member in `frameworks/eventkit/
//! scope.toml`.
//!
//! Registered via [`tswift_core::Interpreter::register_builtin_enum`] so
//! leading-dot spellings (`.thisEvent`, `.fullAccess`) resolve against the
//! parameter/return type, exactly like Charts' `SortOrder`. Because builtin
//! enums are invisible to `Interpreter::registered_keys`, the coverage keys are
//! injected by [`coverage_keys`].

/// One in-scope EventKit enum: its Swift name and its case names (declaration
/// order). Every enum additionally contributes an `init` coverage key for its
/// `init(rawValue:)`.
struct EnumDef {
    name: &'static str,
    cases: &'static [&'static str],
}

/// The in-scope EventKit enums (scope.toml tiers EK1–EK4), less the ObjC
/// option-set masks (`EKEntityMask`, `EKCalendarEventAvailabilityMask`) and the
/// `EKError.Code` domain, which are out of scope.
const ENUMS: &[EnumDef] = &[
    // ── EK1 — store & authorization ────────────────────────────────────
    EnumDef {
        name: "EKAuthorizationStatus",
        cases: &[
            "notDetermined",
            "restricted",
            "denied",
            "fullAccess",
            "writeOnly",
        ],
    },
    EnumDef {
        name: "EKEntityType",
        cases: &["event", "reminder"],
    },
    EnumDef {
        name: "EKSpan",
        cases: &["thisEvent", "futureEvents"],
    },
    // ── EK2 — calendar items ───────────────────────────────────────────
    EnumDef {
        name: "EKEventStatus",
        cases: &["none", "confirmed", "tentative", "canceled"],
    },
    EnumDef {
        name: "EKEventAvailability",
        cases: &["notSupported", "busy", "free", "tentative", "unavailable"],
    },
    EnumDef {
        name: "EKReminderPriority",
        cases: &["none", "high", "medium", "low"],
    },
    // ── EK3 — calendars & sources ──────────────────────────────────────
    EnumDef {
        name: "EKCalendarType",
        cases: &["local", "calDAV", "exchange", "subscription", "birthday"],
    },
    EnumDef {
        name: "EKSourceType",
        cases: &[
            "local",
            "exchange",
            "calDAV",
            "mobileMe",
            "subscribed",
            "birthdays",
        ],
    },
    // ── EK4 — alarms, recurrence, participants ─────────────────────────
    EnumDef {
        name: "EKAlarmType",
        cases: &["display", "audio", "procedure", "email"],
    },
    EnumDef {
        name: "EKAlarmProximity",
        cases: &["none", "enter", "leave"],
    },
    EnumDef {
        name: "EKRecurrenceFrequency",
        cases: &["daily", "weekly", "monthly", "yearly"],
    },
    EnumDef {
        name: "EKParticipantRole",
        cases: &["unknown", "required", "optional", "chair", "nonParticipant"],
    },
    EnumDef {
        name: "EKParticipantStatus",
        cases: &[
            "unknown",
            "pending",
            "accepted",
            "declined",
            "tentative",
            "delegated",
            "completed",
            "inProcess",
        ],
    },
    EnumDef {
        name: "EKParticipantType",
        cases: &["unknown", "person", "room", "resource", "group"],
    },
    EnumDef {
        name: "EKParticipantScheduleStatus",
        cases: &[
            "none",
            "pending",
            "sent",
            "delivered",
            "recipientNotRecognized",
            "noPrivileges",
            "deliveryFailed",
            "cannotDeliver",
            "recipientNotAllowed",
        ],
    },
    EnumDef {
        name: "EKWeekday",
        cases: &[
            "sunday",
            "monday",
            "tuesday",
            "wednesday",
            "thursday",
            "friday",
            "saturday",
        ],
    },
];

/// Register every in-scope EventKit enum on `interp` (already inside the
/// `EventKit` module scope).
pub(crate) fn install(interp: &mut tswift_core::Interpreter<'_>) {
    for def in ENUMS {
        interp.register_builtin_enum(def.name, def.cases);
    }
}

/// Coverage keys (`Type.case` + `Type.init`) for every in-scope enum. Builtin
/// enums do not appear in `Interpreter::registered_keys`, so these are injected
/// by [`crate::registered_keys`].
pub(crate) fn coverage_keys() -> Vec<String> {
    let mut keys = Vec::new();
    for def in ENUMS {
        for case in def.cases {
            keys.push(format!("{}.{}", def.name, case));
        }
        // `init(rawValue:)` → the inventory records this as `init`.
        keys.push(format!("{}.init", def.name));
    }
    keys
}
