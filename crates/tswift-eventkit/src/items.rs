//! EventKit **calendar items**: `EKEvent` and `EKReminder`, their shared
//! `EKCalendarItem` base, the `EKObject` change-tracking base, and the
//! read-only `EKParticipant` value they surface.
//!
//! On device `EKEvent`/`EKReminder` are `EKCalendarItem` subclasses, itself an
//! `EKObject` subclass — three levels of an Objective-C class hierarchy. Builtin
//! receivers in this runtime have **no inheritance** (each class name mints its
//! own [`tswift_core::BuiltinReceiver`]), so we *fold* the base surfaces onto
//! each concrete type: [`install_item_surface`] registers the `EKCalendarItem`
//! + `EKObject` members on both the `EKEvent` and `EKReminder` receivers, and
//!   each constructor seeds the base stored fields alongside the concrete ones.
//!
//! Each object is a [`SwiftValue::Object`] over a [`ClassObj`] (the same
//! reference-with-mutable-state shape as `store.rs`). Settable properties are
//! plain stored fields (get/set flows through the interpreter's generic
//! object-field path); the derived `has*` flags are computed properties
//! registered via [`Interpreter::register_property`]; `addAlarm`/`refresh`/… are
//! intrinsics that mutate the shared `ClassObj` in place.
//!
//! `EKParticipant` is not user-constructible on device (the system populates an
//! event's `organizer`/`attendees`), so a fresh `EKEvent` seeds one organizer +
//! one attendee, letting programs read the participant surface end to end.

use std::cell::RefCell;
use std::rc::Rc;

use tswift_core::{
    Arg, BuiltinReceiver, ClassObj, EnumObj, Interpreter, MethodEntry, Outcome, StdContext,
    StdError, StdResult, SwiftValue,
};

// ── value helpers (shared shapes with store.rs) ────────────────────────────

fn object(class: &str, fields: Vec<(&str, SwiftValue)>) -> SwiftValue {
    SwiftValue::Object(Rc::new(RefCell::new(ClassObj {
        class_name: class.to_string(),
        fields: fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    })))
}

fn enum_val(ty: &str, case: &str) -> SwiftValue {
    SwiftValue::Enum(Rc::new(EnumObj {
        type_name: ty.to_string(),
        case: case.to_string(),
        payload: Vec::new(),
    }))
}

fn array(items: Vec<SwiftValue>) -> SwiftValue {
    SwiftValue::Array(Rc::new(items))
}

fn as_obj(recv: &SwiftValue) -> Option<Rc<RefCell<ClassObj>>> {
    match recv {
        SwiftValue::Object(o) => Some(Rc::clone(o)),
        _ => None,
    }
}

/// Read a field as its array items (empty when absent / nil / not an array).
fn field_items(obj: &Rc<RefCell<ClassObj>>, name: &str) -> Vec<SwiftValue> {
    match obj.borrow().get(name).cloned() {
        Some(SwiftValue::Array(items)) => items.as_ref().clone(),
        _ => Vec::new(),
    }
}

// ── intrinsic adapters ─────────────────────────────────────────────────────

fn method(
    f: fn(&mut dyn StdContext, SwiftValue, Vec<SwiftValue>) -> Result<Outcome, StdError>,
) -> MethodEntry {
    MethodEntry {
        mutating: false,
        func: f,
    }
}

/// The bare function-pointer shape a labeled EventKit intrinsic uses.
type LabeledFn = fn(&mut dyn StdContext, SwiftValue, Vec<Arg>) -> Result<Option<Outcome>, StdError>;

fn labeled(f: LabeledFn) -> tswift_core::LabeledMethodEntry {
    tswift_core::LabeledMethodEntry {
        mutating: false,
        func: f,
    }
}

fn out(result: SwiftValue, receiver: SwiftValue) -> Result<Outcome, StdError> {
    Ok(Outcome { result, receiver })
}

fn handled(result: SwiftValue, receiver: SwiftValue) -> Result<Option<Outcome>, StdError> {
    Ok(Some(Outcome { result, receiver }))
}

// ── registration ───────────────────────────────────────────────────────────

/// Register `EKEvent` / `EKReminder` / `EKParticipant` (already inside the
/// `EventKit` module scope).
pub(crate) fn install(interp: &mut Interpreter<'_>) {
    for class in ["EKEvent", "EKReminder", "EKParticipant"] {
        BuiltinReceiver::register_extension(class);
    }
    let event = BuiltinReceiver::register_extension("EKEvent");
    let reminder = BuiltinReceiver::register_extension("EKReminder");
    let participant = BuiltinReceiver::register_extension("EKParticipant");

    // Constructors.
    interp.register_free_fn("EKEvent", |_, args| Ok(event_init(args)));
    interp.register_free_fn("EKReminder", |_, args| Ok(reminder_init(args)));

    // Shared EKCalendarItem + EKObject surface, folded onto both concrete types.
    install_item_surface(interp, event);
    install_item_surface(interp, reminder);

    // EKEvent-only: compareStartDate(with:).
    interp.register_labeled_intrinsic(event, "compareStartDate", labeled(event_compare));

    // EKParticipant.abRecord(with:) — no ABAddressBook in a headless runtime.
    interp.register_labeled_intrinsic(
        participant,
        "abRecord",
        labeled(|_c, r, _a| handled(SwiftValue::Nil, r)),
    );
}

/// Fold the `EKCalendarItem` + `EKObject` members onto a concrete receiver
/// (builtin receivers have no inheritance, so this runs once per subclass).
fn install_item_surface(interp: &mut Interpreter<'_>, recv: BuiltinReceiver) {
    // EKCalendarItem alarm / recurrence mutators.
    interp.register_intrinsic(recv, "addAlarm", method(item_add_alarm));
    interp.register_intrinsic(recv, "removeAlarm", method(item_remove_alarm));
    interp.register_intrinsic(recv, "addRecurrenceRule", method(item_add_rule));
    interp.register_intrinsic(recv, "removeRecurrenceRule", method(item_remove_rule));

    // EKCalendarItem derived flags (computed — never stored fields).
    interp.register_property(recv, "hasAlarms", |v| has_items(&v, "alarms"));
    interp.register_property(recv, "hasRecurrenceRules", |v| {
        has_items(&v, "recurrenceRules")
    });
    interp.register_property(recv, "hasAttendees", |v| has_items(&v, "attendees"));
    interp.register_property(recv, "hasNotes", has_notes);

    // EKObject change tracking.
    interp.register_intrinsic(
        recv,
        "refresh",
        method(|_c, r, _a| out(SwiftValue::Bool(true), r)),
    );
    interp.register_intrinsic(recv, "reset", method(|_c, r, _a| out(SwiftValue::Void, r)));
    interp.register_intrinsic(
        recv,
        "rollback",
        method(|_c, r, _a| out(SwiftValue::Void, r)),
    );
}

// ── constructors ───────────────────────────────────────────────────────────

/// The `EKCalendarItem` + `EKObject` base stored fields every concrete item
/// carries (folded in, since builtin receivers have no inheritance).
fn base_fields() -> Vec<(&'static str, SwiftValue)> {
    vec![
        // EKCalendarItem
        ("title", SwiftValue::Str(String::new())),
        ("calendar", SwiftValue::Nil),
        ("calendarItemIdentifier", SwiftValue::Str(String::new())),
        ("calendarItemExternalIdentifier", SwiftValue::Nil),
        ("creationDate", SwiftValue::Nil),
        ("lastModifiedDate", SwiftValue::Nil),
        ("timeZone", SwiftValue::Nil),
        ("location", SwiftValue::Nil),
        ("notes", SwiftValue::Nil),
        ("url", SwiftValue::Nil),
        ("alarms", SwiftValue::Nil),
        ("recurrenceRules", SwiftValue::Nil),
        ("attendees", SwiftValue::Nil),
        // EKObject
        ("isNew", SwiftValue::Bool(true)),
        ("hasChanges", SwiftValue::Bool(false)),
    ]
}

/// Pull the store's `defaultCalendarForNewEvents` out of an `eventStore:` arg.
fn default_calendar(args: &[Arg]) -> SwiftValue {
    args.iter()
        .find_map(|a| match &a.value {
            SwiftValue::Object(o) if o.borrow().class_name == "EKEventStore" => {
                o.borrow().get("defaultCalendarForNewEvents").cloned()
            }
            _ => None,
        })
        .unwrap_or(SwiftValue::Nil)
}

fn event_init(args: Vec<Arg>) -> SwiftValue {
    let mut fields = base_fields();
    fields.push(("availability", enum_val("EKEventAvailability", "busy")));
    fields.push(("birthdayContactIdentifier", SwiftValue::Nil));
    fields.push(("birthdayPersonID", SwiftValue::Nil));
    fields.push(("endDate", SwiftValue::Nil));
    fields.push(("eventIdentifier", SwiftValue::Str(String::new())));
    fields.push(("isAllDay", SwiftValue::Bool(false)));
    fields.push(("isDetached", SwiftValue::Bool(false)));
    fields.push(("occurrenceDate", SwiftValue::Nil));
    fields.push(("organizer", participant("Organizer", "chair")));
    fields.push(("startDate", SwiftValue::Nil));
    fields.push(("status", enum_val("EKEventStatus", "none")));
    fields.push(("structuredLocation", SwiftValue::Nil));

    let obj = object("EKEvent", fields);
    // Seed the system-populated organizer/attendee + owning calendar.
    if let Some(o) = as_obj(&obj) {
        o.borrow_mut().set(
            "attendees",
            array(vec![participant("Attendee", "required")]),
        );
        o.borrow_mut().set("calendar", default_calendar(&args));
    }
    obj
}

fn reminder_init(args: Vec<Arg>) -> SwiftValue {
    let mut fields = base_fields();
    fields.push(("completionDate", SwiftValue::Nil));
    fields.push(("dueDateComponents", SwiftValue::Nil));
    fields.push(("isCompleted", SwiftValue::Bool(false)));
    fields.push(("priority", SwiftValue::int(0)));
    fields.push(("startDateComponents", SwiftValue::Nil));

    let obj = object("EKReminder", fields);
    if let Some(o) = as_obj(&obj) {
        o.borrow_mut().set("calendar", default_calendar(&args));
    }
    obj
}

/// A read-only `EKParticipant` with the given display name and role.
fn participant(name: &str, role: &str) -> SwiftValue {
    object(
        "EKParticipant",
        vec![
            ("name", SwiftValue::Str(name.to_string())),
            ("url", SwiftValue::Nil),
            ("isCurrentUser", SwiftValue::Bool(false)),
            ("participantRole", enum_val("EKParticipantRole", role)),
            (
                "participantStatus",
                enum_val("EKParticipantStatus", "accepted"),
            ),
            ("participantType", enum_val("EKParticipantType", "person")),
            ("contactPredicate", SwiftValue::Nil),
        ],
    )
}

// ── EKCalendarItem methods ─────────────────────────────────────────────────

/// Append `arg` to the receiver's optional-array `field`, materialising the
/// array on first insert.
fn append_field(recv: &SwiftValue, field: &str, value: SwiftValue) {
    if let Some(obj) = as_obj(recv) {
        let mut items = field_items(&obj, field);
        items.push(value);
        obj.borrow_mut().set(field, array(items));
    }
}

/// Remove elements equal to `value` from the receiver's `field` array; reset the
/// field to nil once empty (mirrors EventKit's `[T]?` back to `nil`).
fn remove_field(recv: &SwiftValue, field: &str, value: &SwiftValue) {
    if let Some(obj) = as_obj(recv) {
        let remaining: Vec<SwiftValue> = field_items(&obj, field)
            .into_iter()
            .filter(|existing| !same_ref(existing, value))
            .collect();
        let stored = if remaining.is_empty() {
            SwiftValue::Nil
        } else {
            array(remaining)
        };
        obj.borrow_mut().set(field, stored);
    }
}

/// Reference identity for two object values (EventKit types are classes).
fn same_ref(a: &SwiftValue, b: &SwiftValue) -> bool {
    match (a, b) {
        (SwiftValue::Object(x), SwiftValue::Object(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

fn item_add_alarm(
    _c: &mut dyn StdContext,
    r: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if let Some(a) = args.into_iter().next() {
        append_field(&r, "alarms", a);
    }
    out(SwiftValue::Void, r)
}

fn item_remove_alarm(
    _c: &mut dyn StdContext,
    r: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if let Some(a) = args.first() {
        remove_field(&r, "alarms", a);
    }
    out(SwiftValue::Void, r)
}

fn item_add_rule(
    _c: &mut dyn StdContext,
    r: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if let Some(a) = args.into_iter().next() {
        append_field(&r, "recurrenceRules", a);
    }
    out(SwiftValue::Void, r)
}

fn item_remove_rule(
    _c: &mut dyn StdContext,
    r: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if let Some(a) = args.first() {
        remove_field(&r, "recurrenceRules", a);
    }
    out(SwiftValue::Void, r)
}

// ── computed flags ─────────────────────────────────────────────────────────

fn has_items(value: &SwiftValue, field: &str) -> StdResult {
    let present = as_obj(value)
        .map(|o| !field_items(&o, field).is_empty())
        .unwrap_or(false);
    Ok(SwiftValue::Bool(present))
}

fn has_notes(value: SwiftValue) -> StdResult {
    let present = matches!(
        as_obj(&value).and_then(|o| o.borrow().get("notes").cloned()),
        Some(SwiftValue::Str(s)) if !s.is_empty()
    );
    Ok(SwiftValue::Bool(present))
}

// ── EKEvent.compareStartDate(with:) ────────────────────────────────────────

/// The `_timeIntervalSinceReferenceDate` seconds inside a Foundation `Date`
/// value, or `None` for a nil / non-Date field.
fn date_seconds(value: &SwiftValue) -> Option<f64> {
    match value {
        SwiftValue::Struct(obj) if obj.type_name == "Date" => {
            match obj.get("_timeIntervalSinceReferenceDate") {
                Some(SwiftValue::Double(s)) => Some(*s),
                Some(SwiftValue::Int(s)) => Some(s.raw as f64),
                _ => None,
            }
        }
        _ => None,
    }
}

fn start_seconds(item: &SwiftValue) -> Option<f64> {
    as_obj(item)
        .and_then(|o| o.borrow().get("startDate").cloned())
        .as_ref()
        .and_then(date_seconds)
}

/// `compareStartDate(with:)` — order two events by `startDate`, returning a
/// Foundation `ComparisonResult`. Missing dates sort as equal.
fn event_compare(
    _c: &mut dyn StdContext,
    r: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let other = args.into_iter().next().map(|a| a.value);
    let lhs = start_seconds(&r);
    let rhs = other.as_ref().and_then(start_seconds);
    let case = match (lhs, rhs) {
        (Some(l), Some(rr)) if l < rr => "orderedAscending",
        (Some(l), Some(rr)) if l > rr => "orderedDescending",
        _ => "orderedSame",
    };
    handled(enum_val("ComparisonResult", case), r)
}

// ── coverage ───────────────────────────────────────────────────────────────

/// Coverage keys for every in-scope calendar-item member (see
/// `frameworks/eventkit/scope.toml`, tier EK2 + the EK4 `EKObject`/
/// `EKParticipant` bases).
pub(crate) fn coverage_keys() -> Vec<String> {
    let groups: &[(&str, &[&str])] = &[
        (
            "EKCalendarItem",
            &[
                "addAlarm",
                "addRecurrenceRule",
                "removeAlarm",
                "removeRecurrenceRule",
                "alarms",
                "attendees",
                "calendar",
                "calendarItemExternalIdentifier",
                "calendarItemIdentifier",
                "creationDate",
                "hasAlarms",
                "hasAttendees",
                "hasNotes",
                "hasRecurrenceRules",
                "lastModifiedDate",
                "location",
                "notes",
                "recurrenceRules",
                "timeZone",
                "title",
                "url",
            ],
        ),
        (
            "EKEvent",
            &[
                "init",
                "compareStartDate",
                "refresh",
                "availability",
                "birthdayContactIdentifier",
                "birthdayPersonID",
                "endDate",
                "eventIdentifier",
                "isAllDay",
                "isDetached",
                "occurrenceDate",
                "organizer",
                "startDate",
                "status",
                "structuredLocation",
            ],
        ),
        (
            "EKReminder",
            &[
                "init",
                "completionDate",
                "dueDateComponents",
                "isCompleted",
                "priority",
                "startDateComponents",
            ],
        ),
        (
            "EKObject",
            &["refresh", "reset", "rollback", "hasChanges", "isNew"],
        ),
        (
            "EKParticipant",
            &[
                "abRecord",
                "contactPredicate",
                "isCurrentUser",
                "name",
                "participantRole",
                "participantStatus",
                "participantType",
                "url",
            ],
        ),
    ];
    let mut keys = Vec::new();
    for (ty, members) in groups {
        for m in *members {
            keys.push(format!("{ty}.{m}"));
        }
    }
    keys
}
