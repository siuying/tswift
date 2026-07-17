//! The headless, in-memory EventKit store: `EKEventStore` plus the two
//! reference types it owns and hands back, `EKCalendar` and `EKSource`.
//!
//! On device `EKEventStore` reads and writes the user's Calendar/Reminders
//! database through the system daemon behind a permission prompt. There is no
//! browser/wasm equivalent (see `frameworks/eventkit/scope.toml`), so — like
//! SwiftData's in-memory `ModelConfiguration` — we model the store as a plain
//! reference object holding in-memory arrays of calendars/events/reminders.
//!
//! Every EventKit object is a [`SwiftValue::Object`] over a [`ClassObj`]:
//!   * **Read-only / settable properties** (`title`, `sources`,
//!     `eventStoreIdentifier`, …) are ordinary stored fields, seeded at
//!     construction, so get/set flows through the interpreter's generic
//!     object-field path — no per-property registration needed.
//!   * **Methods** (`save`, `requestFullAccessToEvents`, `calendars(for:)`, …)
//!     are registered as intrinsics on the `Extension` receiver minted for the
//!     class name; they mutate the shared `ClassObj` in place through its
//!     `RefCell` (reference semantics), matching core's `URLSessionDataTask`.
//!
//! Permissions resolve deterministically: there is no UI to prompt, so
//! `requestFullAccessToEvents`/`Reminders` and `requestWriteOnlyAccessToEvents`
//! grant synchronously (both the completion-handler and `async` spellings), and
//! `authorizationStatus(for:)` reflects the last granted level per entity type
//! via a process-local map cleared on interpreter teardown.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use tswift_core::{
    Arg, BuiltinReceiver, ClassObj, EnumObj, Interpreter, MethodEntry, Outcome, StdContext,
    StdError, SwiftValue,
};

thread_local! {
    /// Authorization level granted per entity-type case name (`"event"` /
    /// `"reminder"`), set by the `request*` methods and read by the
    /// `authorizationStatus(for:)` class method. Cleared on interpreter
    /// teardown so status never leaks across programs in one process.
    static GRANTED: RefCell<HashMap<String, &'static str>> = RefCell::new(HashMap::new());
    /// Monotonic id counter minting deterministic EventKit identifiers.
    static NEXT_ID: RefCell<u64> = const { RefCell::new(1) };
}

fn next_id() -> u64 {
    NEXT_ID.with(|c| {
        let mut c = c.borrow_mut();
        let id = *c;
        *c += 1;
        id
    })
}

/// Build a `SwiftValue::Object` for `class` with the given seeded fields.
fn object(class: &str, fields: Vec<(&str, SwiftValue)>) -> SwiftValue {
    SwiftValue::Object(Rc::new(RefCell::new(ClassObj {
        class_name: class.to_string(),
        fields: fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    })))
}

/// Build an `EK…` enum case value (`EKSourceType.local`, …).
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

/// Read the `EKEntityType` case name out of the (labeled or positional) args of
/// a permission/`calendars(for:)` call, defaulting to `"event"`.
fn entity_case(args: &[Arg]) -> String {
    for a in args {
        if let SwiftValue::Enum(e) = &a.value {
            if e.type_name == "EKEntityType" {
                return e.case.clone();
            }
        }
    }
    "event".to_string()
}

/// Grant `level` for the entity type named in `args`, then resolve the call as
/// either the completion-handler form (invoke `(Bool, Error?) -> Void`) or the
/// `async throws -> Bool` form (return `true`).
fn grant(
    ctx: &mut dyn StdContext,
    args: Vec<Arg>,
    entity: &str,
    level: &'static str,
) -> Result<Option<Outcome>, StdError> {
    GRANTED.with(|g| g.borrow_mut().insert(entity.to_string(), level));
    // If a completion handler was supplied, invoke it with (granted, nil).
    for a in &args {
        if let SwiftValue::Closure(id) = a.value {
            ctx.call_closure(id, vec![SwiftValue::Bool(true), SwiftValue::Nil])?;
            return Ok(Some(Outcome {
                result: SwiftValue::Void,
                receiver: SwiftValue::Void,
            }));
        }
    }
    // Otherwise the `async throws -> Bool` spelling: report success.
    Ok(Some(Outcome {
        result: SwiftValue::Bool(true),
        receiver: SwiftValue::Void,
    }))
}

/// Borrow the receiver's `ClassObj` (returns `None` for non-objects).
fn as_obj(recv: &SwiftValue) -> Option<Rc<RefCell<ClassObj>>> {
    match recv {
        SwiftValue::Object(o) => Some(Rc::clone(o)),
        _ => None,
    }
}

/// Read a field as an array's items (empty when absent / not an array).
fn field_array(obj: &Rc<RefCell<ClassObj>>, name: &str) -> Vec<SwiftValue> {
    match obj.borrow().get(name).cloned() {
        Some(SwiftValue::Array(items)) => items.as_ref().clone(),
        _ => Vec::new(),
    }
}

/// Clone a field value out of an object (`None` for non-objects / absent field).
fn field_of(item: &SwiftValue, name: &str) -> Option<SwiftValue> {
    as_obj(item).and_then(|o| o.borrow().get(name).cloned())
}

/// Register the EKEventStore / EKCalendar / EKSource surface (already inside the
/// `EventKit` module scope).
pub(crate) fn install(interp: &mut Interpreter<'_>) {
    // Reset granted-authorization state deterministically at teardown.
    interp.register_finalizer(Box::new(|_ctx| {
        GRANTED.with(|g| g.borrow_mut().clear());
    }));

    for class in ["EKEventStore", "EKCalendar", "EKSource"] {
        BuiltinReceiver::register_extension(class);
    }
    let store = BuiltinReceiver::register_extension("EKEventStore");
    let source = BuiltinReceiver::register_extension("EKSource");

    // ── Constructors ──────────────────────────────────────────────────
    interp.register_free_fn("EKEventStore", |_, _| Ok(store_init()));
    interp.register_free_fn("EKCalendar", |_, args| Ok(calendar_init(args)));

    // ── EKEventStore permissions ──────────────────────────────────────
    interp.register_labeled_intrinsic(
        store,
        "requestFullAccessToEvents",
        labeled(|ctx, _r, a| grant(ctx, a, "event", "fullAccess")),
    );
    interp.register_labeled_intrinsic(
        store,
        "requestFullAccessToReminders",
        labeled(|ctx, _r, a| grant(ctx, a, "reminder", "fullAccess")),
    );
    interp.register_labeled_intrinsic(
        store,
        "requestWriteOnlyAccessToEvents",
        labeled(|ctx, _r, a| grant(ctx, a, "event", "writeOnly")),
    );
    interp.register_labeled_intrinsic(
        store,
        "requestAccess",
        labeled(|ctx, _r, a| {
            let entity = entity_case(&a);
            grant(ctx, a.clone(), &entity, "fullAccess")
        }),
    );
    interp.register_static(store, "authorizationStatus", |_ctx, args| {
        let entity = entity_case(&args);
        let level = GRANTED.with(|g| g.borrow().get(&entity).copied().unwrap_or("notDetermined"));
        Ok(enum_val("EKAuthorizationStatus", level))
    });

    // ── EKEventStore CRUD & accessors ─────────────────────────────────
    interp.register_labeled_intrinsic(store, "saveCalendar", labeled(store_save_calendar));
    interp.register_labeled_intrinsic(store, "removeCalendar", labeled(store_remove_calendar));
    interp.register_labeled_intrinsic(store, "save", labeled(store_save_item));
    interp.register_labeled_intrinsic(store, "remove", labeled(store_remove_item));
    interp.register_labeled_intrinsic(store, "calendars", labeled(store_calendars));
    interp.register_labeled_intrinsic(store, "calendar", labeled(store_calendar_lookup));
    interp.register_labeled_intrinsic(store, "event", labeled(store_event_lookup));
    interp.register_labeled_intrinsic(store, "calendarItem", labeled(store_item_lookup));
    interp.register_labeled_intrinsic(store, "calendarItems", labeled(store_items_lookup));
    interp.register_labeled_intrinsic(store, "source", labeled(store_source_lookup));
    interp.register_intrinsic(store, "commit", method(|_c, r, _a| ok_void(r)));
    interp.register_intrinsic(store, "reset", method(store_reset));
    interp.register_intrinsic(
        store,
        "refreshSourcesIfNecessary",
        method(|_c, r, _a| ok_void(r)),
    );
    interp.register_intrinsic(
        store,
        "defaultCalendarForNewReminders",
        method(|_c, r, _a| {
            let cal = field_of(&r, "defaultCalendarForNewEvents").unwrap_or(SwiftValue::Nil);
            Ok(Outcome {
                result: cal,
                receiver: r,
            })
        }),
    );

    // ── EKSource ──────────────────────────────────────────────────────
    interp.register_labeled_intrinsic(
        source,
        "calendars",
        labeled(|_c, r, _a| {
            let cals = as_obj(&r)
                .map(|o| field_array(&o, "_calendars"))
                .unwrap_or_default();
            Ok(Some(Outcome {
                result: array(cals),
                receiver: r,
            }))
        }),
    );
}

// ── intrinsic adapters ────────────────────────────────────────────────────

/// Wrap a positional-method closure into a [`MethodEntry`] (non-mutating:
/// objects are reference types, mutated in place through their `RefCell`).
fn method(
    f: fn(&mut dyn StdContext, SwiftValue, Vec<SwiftValue>) -> Result<Outcome, StdError>,
) -> MethodEntry {
    MethodEntry {
        mutating: false,
        func: f,
    }
}

/// The bare function-pointer shape a label-aware EventKit intrinsic uses.
type LabeledFn = fn(&mut dyn StdContext, SwiftValue, Vec<Arg>) -> Result<Option<Outcome>, StdError>;

/// Wrap a label-aware method closure into a [`LabeledMethodEntry`].
fn labeled(f: LabeledFn) -> tswift_core::LabeledMethodEntry {
    tswift_core::LabeledMethodEntry {
        mutating: false,
        func: f,
    }
}

fn ok_void(receiver: SwiftValue) -> Result<Outcome, StdError> {
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver,
    })
}

fn handled(result: SwiftValue, receiver: SwiftValue) -> Result<Option<Outcome>, StdError> {
    Ok(Some(Outcome { result, receiver }))
}

// ── constructors ──────────────────────────────────────────────────────────

fn store_init() -> SwiftValue {
    let src = source_local();
    let cal = new_calendar("Calendar", src.clone());
    object(
        "EKEventStore",
        vec![
            (
                "eventStoreIdentifier",
                SwiftValue::Str(format!("EKEventStore-{}", next_id())),
            ),
            ("sources", array(vec![src])),
            ("delegateSources", array(vec![])),
            ("defaultCalendarForNewEvents", cal.clone()),
            ("_calendars", array(vec![cal])),
            ("_events", array(vec![])),
            ("_reminders", array(vec![])),
        ],
    )
}

/// The single built-in local `EKSource` every fresh store exposes.
fn source_local() -> SwiftValue {
    object(
        "EKSource",
        vec![
            ("title", SwiftValue::Str("Local".to_string())),
            ("sourceType", enum_val("EKSourceType", "local")),
            (
                "sourceIdentifier",
                SwiftValue::Str(format!("EKSource-{}", next_id())),
            ),
            ("isDelegate", SwiftValue::Bool(false)),
            ("_calendars", array(vec![])),
        ],
    )
}

fn new_calendar(title: &str, source: SwiftValue) -> SwiftValue {
    object(
        "EKCalendar",
        vec![
            ("title", SwiftValue::Str(title.to_string())),
            ("type", enum_val("EKCalendarType", "local")),
            (
                "calendarIdentifier",
                SwiftValue::Str(format!("EKCalendar-{}", next_id())),
            ),
            ("allowsContentModifications", SwiftValue::Bool(true)),
            ("isImmutable", SwiftValue::Bool(false)),
            ("isSubscribed", SwiftValue::Bool(false)),
            ("source", source),
            ("cgColor", SwiftValue::Nil),
            ("allowedEntityTypes", SwiftValue::int(3)),
            ("supportedEventAvailabilities", SwiftValue::int(0)),
        ],
    )
}

/// `EKCalendar(for:eventStore:)` — seed a fresh, empty, modifiable calendar. The
/// `for:` entity type and `eventStore:` are accepted for signature fidelity; the
/// new calendar takes the store's default local source when one is supplied.
fn calendar_init(args: Vec<Arg>) -> SwiftValue {
    let source = args
        .iter()
        .find_map(|a| match &a.value {
            SwiftValue::Object(o) if o.borrow().class_name == "EKEventStore" => {
                match o.borrow().get("sources").cloned() {
                    Some(SwiftValue::Array(items)) => items.first().cloned(),
                    _ => None,
                }
            }
            _ => None,
        })
        .unwrap_or(SwiftValue::Nil);
    new_calendar("", source)
}

// ── EKEventStore methods ──────────────────────────────────────────────────

/// First positional (unlabeled) argument value, if any.
fn first_positional(args: &[Arg]) -> Option<SwiftValue> {
    args.iter()
        .find(|a| a.label.is_none())
        .map(|a| a.value.clone())
}

/// Push `item` onto the store's `field` array unless an object with the same
/// `id_field` identifier is already present; return whether it was appended.
fn upsert(store: &Rc<RefCell<ClassObj>>, field: &str, id_field: &str, item: SwiftValue) {
    let id = field_of(&item, id_field);
    let mut items = field_array(store, field);
    let exists = items
        .iter()
        .any(|existing| field_of(existing, id_field) == id);
    if !exists {
        items.push(item);
        store.borrow_mut().set(field, array(items));
    }
}

fn remove_by_id(store: &Rc<RefCell<ClassObj>>, field: &str, id_field: &str, item: &SwiftValue) {
    let id = field_of(item, id_field);
    let items: Vec<SwiftValue> = field_array(store, field)
        .into_iter()
        .filter(|existing| field_of(existing, id_field) != id)
        .collect();
    store.borrow_mut().set(field, array(items));
}

fn store_save_calendar(
    _c: &mut dyn StdContext,
    r: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    if let (Some(store), Some(cal)) = (as_obj(&r), first_positional(&args)) {
        upsert(&store, "_calendars", "calendarIdentifier", cal);
    }
    handled(SwiftValue::Void, r)
}

fn store_remove_calendar(
    _c: &mut dyn StdContext,
    r: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    if let (Some(store), Some(cal)) = (as_obj(&r), first_positional(&args)) {
        remove_by_id(&store, "_calendars", "calendarIdentifier", &cal);
    }
    handled(SwiftValue::Void, r)
}

/// `save(_:span:)` for events/reminders — dispatch to the store array matching
/// the item's class, assigning its identifier on first save.
fn store_save_item(
    _c: &mut dyn StdContext,
    r: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    if let (Some(store), Some(item)) = (as_obj(&r), first_positional(&args)) {
        if let Some(obj) = as_obj(&item) {
            let class_name = obj.borrow().class_name.clone();
            let (field, id_field) = match class_name.as_str() {
                "EKReminder" => ("_reminders", "calendarItemIdentifier"),
                _ => ("_events", "eventIdentifier"),
            };
            let unset = matches!(obj.borrow().get(id_field), None | Some(SwiftValue::Nil))
                || matches!(obj.borrow().get(id_field), Some(SwiftValue::Str(s)) if s.is_empty());
            if unset {
                obj.borrow_mut()
                    .set(id_field, SwiftValue::Str(format!("{field}-{}", next_id())));
            }
            // Persisting clears the item's newness flag (EKObject.isNew).
            if obj.borrow().get("isNew").is_some() {
                obj.borrow_mut().set("isNew", SwiftValue::Bool(false));
            }
            upsert(&store, field, id_field, item);
        }
    }
    handled(SwiftValue::Void, r)
}

fn store_remove_item(
    _c: &mut dyn StdContext,
    r: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    if let (Some(store), Some(item)) = (as_obj(&r), first_positional(&args)) {
        if let Some(obj) = as_obj(&item) {
            let class_name = obj.borrow().class_name.clone();
            let (field, id_field) = match class_name.as_str() {
                "EKReminder" => ("_reminders", "calendarItemIdentifier"),
                _ => ("_events", "eventIdentifier"),
            };
            remove_by_id(&store, field, id_field, &item);
        }
    }
    handled(SwiftValue::Void, r)
}

fn store_calendars(
    _c: &mut dyn StdContext,
    r: SwiftValue,
    _args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let cals = as_obj(&r)
        .map(|o| field_array(&o, "_calendars"))
        .unwrap_or_default();
    handled(array(cals), r)
}

/// Generic `<thing>(withIdentifier:)` lookup over a store array by id field.
fn lookup_by_id(store: &SwiftValue, field: &str, id_field: &str, args: &[Arg]) -> SwiftValue {
    let Some(store) = as_obj(store) else {
        return SwiftValue::Nil;
    };
    let wanted = args.iter().find_map(|a| match &a.value {
        SwiftValue::Str(s) => Some(s.clone()),
        _ => None,
    });
    let Some(wanted) = wanted else {
        return SwiftValue::Nil;
    };
    field_array(&store, field)
        .into_iter()
        .find(|item| {
            matches!(
                field_of(item, id_field),
                Some(SwiftValue::Str(s)) if s == wanted
            )
        })
        .unwrap_or(SwiftValue::Nil)
}

fn store_calendar_lookup(
    _c: &mut dyn StdContext,
    r: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let found = lookup_by_id(&r, "_calendars", "calendarIdentifier", &args);
    handled(found, r)
}

fn store_event_lookup(
    _c: &mut dyn StdContext,
    r: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let found = lookup_by_id(&r, "_events", "eventIdentifier", &args);
    handled(found, r)
}

fn store_item_lookup(
    _c: &mut dyn StdContext,
    r: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let found = lookup_by_id(&r, "_reminders", "calendarItemIdentifier", &args);
    handled(found, r)
}

fn store_items_lookup(
    _c: &mut dyn StdContext,
    r: SwiftValue,
    _args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    handled(array(vec![]), r)
}

fn store_source_lookup(
    _c: &mut dyn StdContext,
    r: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let found = lookup_by_id(&r, "sources", "sourceIdentifier", &args);
    handled(found, r)
}

/// `reset()` — discard unsaved changes; our in-memory model has no dirty tree,
/// so this is a no-op that echoes the store back.
fn store_reset(
    _c: &mut dyn StdContext,
    r: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    ok_void(r)
}

/// Coverage keys for the store surface: every in-scope `EKEventStore` /
/// `EKCalendar` / `EKSource` member (see `frameworks/eventkit/scope.toml`).
pub(crate) fn coverage_keys() -> Vec<String> {
    let mut keys = Vec::new();
    let groups: &[(&str, &[&str])] = &[
        (
            "EKEventStore",
            &[
                "init",
                "authorizationStatus",
                "requestFullAccessToEvents",
                "requestFullAccessToReminders",
                "requestWriteOnlyAccessToEvents",
                "requestAccess",
                "save",
                "remove",
                "saveCalendar",
                "removeCalendar",
                "commit",
                "reset",
                "refreshSourcesIfNecessary",
                "calendars",
                "calendar",
                "event",
                "calendarItem",
                "calendarItems",
                "source",
                "sources",
                "delegateSources",
                "defaultCalendarForNewEvents",
                "defaultCalendarForNewReminders",
                "eventStoreIdentifier",
            ],
        ),
        (
            "EKCalendar",
            &[
                "init",
                "title",
                "type",
                "calendarIdentifier",
                "allowsContentModifications",
                "isImmutable",
                "isSubscribed",
                "source",
                "cgColor",
                "allowedEntityTypes",
                "supportedEventAvailabilities",
            ],
        ),
        (
            "EKSource",
            &[
                "title",
                "sourceType",
                "sourceIdentifier",
                "isDelegate",
                "calendars",
            ],
        ),
    ];
    for (ty, members) in groups {
        for m in *members {
            keys.push(format!("{ty}.{m}"));
        }
    }
    keys
}
