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
//! Registered via [`tswift_core::Interpreter::register_builtin_enum_with_raw`] so
//! leading-dot spellings (`.thisEvent`, `.fullAccess`) resolve against the
//! parameter/return type, exactly like Charts' `SortOrder`. Because builtin
//! enums are invisible to `Interpreter::registered_keys`, the coverage keys are
//! injected by [`coverage_keys`].

/// One in-scope EventKit enum: its Swift name and its cases with their
/// NS_ENUM-backed raw values in declaration order. Every enum additionally
/// contributes an `init` coverage key for its `init(rawValue:)`.
struct EnumDef {
    name: &'static str,
    cases: &'static [(&'static str, i128)],
}

/// The in-scope EventKit enums (scope.toml tiers EK1–EK4), less the ObjC
/// option-set masks (`EKEntityMask`, `EKCalendarEventAvailabilityMask`) and the
/// `EKError.Code` domain, which are out of scope.
const ENUMS: &[EnumDef] = &[
    // ── EK1 — store & authorization ────────────────────────────────────
    EnumDef {
        name: "EKAuthorizationStatus",
        cases: &[
            ("notDetermined", 0),
            ("restricted", 1),
            ("denied", 2),
            ("fullAccess", 3),
            ("writeOnly", 4),
            // Deprecated alias of `.fullAccess`, kept for source fidelity.
            ("authorized", 3),
        ],
    },
    EnumDef {
        name: "EKEntityType",
        cases: &[("event", 0), ("reminder", 1)],
    },
    EnumDef {
        name: "EKSpan",
        cases: &[("thisEvent", 0), ("futureEvents", 1)],
    },
    // ── EK2 — calendar items ───────────────────────────────────────────
    EnumDef {
        name: "EKEventStatus",
        cases: &[
            ("none", 0),
            ("confirmed", 1),
            ("tentative", 2),
            ("canceled", 3),
        ],
    },
    EnumDef {
        name: "EKEventAvailability",
        cases: &[
            ("notSupported", -1),
            ("busy", 0),
            ("free", 1),
            ("tentative", 2),
            ("unavailable", 3),
        ],
    },
    EnumDef {
        name: "EKReminderPriority",
        cases: &[("none", 0), ("high", 1), ("medium", 5), ("low", 9)],
    },
    // ── EK3 — calendars & sources ──────────────────────────────────────
    EnumDef {
        name: "EKCalendarType",
        cases: &[
            ("local", 0),
            ("calDAV", 1),
            ("exchange", 2),
            ("subscription", 3),
            ("birthday", 4),
        ],
    },
    EnumDef {
        name: "EKSourceType",
        cases: &[
            ("local", 0),
            ("exchange", 1),
            ("calDAV", 2),
            ("mobileMe", 3),
            ("subscribed", 4),
            ("birthdays", 5),
        ],
    },
    // ── EK4 — alarms, recurrence, participants ─────────────────────────
    EnumDef {
        name: "EKAlarmType",
        cases: &[("display", 0), ("audio", 1), ("procedure", 2), ("email", 3)],
    },
    EnumDef {
        name: "EKAlarmProximity",
        cases: &[("none", 0), ("enter", 1), ("leave", 2)],
    },
    EnumDef {
        name: "EKRecurrenceFrequency",
        cases: &[("daily", 0), ("weekly", 1), ("monthly", 2), ("yearly", 3)],
    },
    EnumDef {
        name: "EKParticipantRole",
        cases: &[
            ("unknown", 0),
            ("required", 1),
            ("optional", 2),
            ("chair", 3),
            ("nonParticipant", 4),
        ],
    },
    EnumDef {
        name: "EKParticipantStatus",
        cases: &[
            ("unknown", 0),
            ("pending", 1),
            ("accepted", 2),
            ("declined", 3),
            ("tentative", 4),
            ("delegated", 5),
            ("completed", 6),
            ("inProcess", 7),
        ],
    },
    EnumDef {
        name: "EKParticipantType",
        cases: &[
            ("unknown", 0),
            ("person", 1),
            ("room", 2),
            ("resource", 3),
            ("group", 4),
        ],
    },
    EnumDef {
        name: "EKParticipantScheduleStatus",
        cases: &[
            ("none", 0),
            ("pending", 1),
            ("sent", 2),
            ("delivered", 3),
            ("recipientNotRecognized", 4),
            ("noPrivileges", 5),
            ("deliveryFailed", 6),
            ("cannotDeliver", 7),
            ("recipientNotAllowed", 8),
        ],
    },
    EnumDef {
        name: "EKWeekday",
        cases: &[
            ("sunday", 1),
            ("monday", 2),
            ("tuesday", 3),
            ("wednesday", 4),
            ("thursday", 5),
            ("friday", 6),
            ("saturday", 7),
            // Deprecated uppercase constant spellings (EKSunday…EKSaturday).
            ("EKSunday", 1),
            ("EKMonday", 2),
            ("EKTuesday", 3),
            ("EKWednesday", 4),
            ("EKThursday", 5),
            ("EKFriday", 6),
            ("EKSaturday", 7),
        ],
    },
];

/// Register every in-scope EventKit enum on `interp` (already inside the
/// `EventKit` module scope).
pub(crate) fn install(interp: &mut tswift_core::Interpreter<'_>) {
    for def in ENUMS {
        interp.register_builtin_enum_with_raw(def.name, def.cases);
    }
}

/// Coverage keys (`Type.case` + `Type.init`) for every in-scope enum. Builtin
/// enums do not appear in `Interpreter::registered_keys`, so these are injected
/// by [`crate::registered_keys`].
pub(crate) fn coverage_keys() -> Vec<String> {
    let mut keys = Vec::new();
    for def in ENUMS {
        for (case, _) in def.cases {
            keys.push(format!("{}.{}", def.name, case));
        }
        // `init(rawValue:)` → the inventory records this as `init`.
        keys.push(format!("{}.init", def.name));
    }
    keys
}
