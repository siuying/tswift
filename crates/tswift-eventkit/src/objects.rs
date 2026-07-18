//! In-scope EventKit **value objects** (EK4 leaves): the pure data-holder
//! reference types `EKAlarm`, `EKStructuredLocation`, `EKRecurrenceEnd`,
//! `EKRecurrenceDayOfWeek`, and `EKRecurrenceRule`.
//!
//! On device these are `NSObject` subclasses (reference types) that carry
//! mutable stored properties. We model each as a `SwiftValue::Object` over a
//! [`ClassObj`] whose fields are seeded with sensible defaults at construction,
//! so property get/set flows through the interpreter's **generic** object-field
//! path (no `ClassDef` needed — see `tswift_core::interp::storage`, which reads
//! a present field directly and writes via `ClassObj::set`). This is the same
//! reference-with-mutable-state shape `URLSessionDataTask` uses in core.
//!
//! Constructors are registered as free functions (`EKAlarm(relativeOffset:)`,
//! …); each seeds the type's default fields, then overlays whatever labeled /
//! positional init arguments arrived. The precise Objective-C initializer
//! spellings all collapse to a single `init` coverage key (see
//! `tools/framework-inventory/coverage.py`), so one lenient constructor per
//! type faithfully covers every `init(...)` form.

use std::rc::Rc;

use tswift_core::{Arg, ClassObj, EnumObj, Interpreter, SwiftValue};

/// One EventKit value-object type: its class name, its stored properties (with
/// default seed values), the labeled-init aliases (`arg label -> field`), and
/// the field names positional (`_`) init arguments fill, in order.
struct ObjectSpec {
    class_name: &'static str,
    /// Stored properties, in declaration order, each with a default seed.
    fields: &'static [(&'static str, Default)],
    /// Labeled-init argument aliases: `(argument label, target field)`.
    aliases: &'static [(&'static str, &'static str)],
    /// Fields filled by positional (`_:`) init arguments, in order.
    positional: &'static [&'static str],
}

/// The default seed for a stored property. Kept tiny — just the handful of
/// primitive shapes EventKit's value-object properties use.
#[derive(Clone, Copy)]
enum Default {
    /// Optional / not-yet-set (Date?, CLLocation?, [T]?, EK…? …) → `nil`.
    Nil,
    /// `String` → `""`.
    Str,
    /// `Int` → the given value (interval defaults to 1, counts to 0).
    Int(i128),
    /// `TimeInterval`/`CLLocationDistance` (`Double`) → `0`.
    Double,
    /// An enum-typed property, seeded with a concrete default case
    /// (`EKAlarmProximity.none`, `EKRecurrenceFrequency.daily`, …).
    Enum(&'static str, &'static str),
}

impl Default {
    fn seed(self) -> SwiftValue {
        match self {
            Default::Nil => SwiftValue::Nil,
            Default::Str => SwiftValue::Str(String::new()),
            Default::Int(v) => SwiftValue::int(v),
            Default::Double => SwiftValue::Double(0.0),
            Default::Enum(ty, case) => SwiftValue::Enum(Rc::new(EnumObj {
                type_name: ty.to_string(),
                case: case.to_string(),
                payload: Vec::new(),
            })),
        }
    }
}

/// Every value-object type this module registers.
const SPECS: &[ObjectSpec] = &[
    // ── EKAlarm ────────────────────────────────────────────────────────
    // init(absoluteDate:) / init(relativeOffset:)
    ObjectSpec {
        class_name: "EKAlarm",
        fields: &[
            ("absoluteDate", Default::Nil),
            ("relativeOffset", Default::Double),
            ("proximity", Default::Enum("EKAlarmProximity", "none")),
            ("structuredLocation", Default::Nil),
        ],
        aliases: &[
            ("absoluteDate", "absoluteDate"),
            ("relativeOffset", "relativeOffset"),
        ],
        positional: &[],
    },
    // ── EKStructuredLocation ───────────────────────────────────────────
    // init(title:) / init(mapItem:)
    ObjectSpec {
        class_name: "EKStructuredLocation",
        fields: &[
            ("title", Default::Str),
            ("geoLocation", Default::Nil),
            ("radius", Default::Double),
        ],
        aliases: &[("title", "title")],
        positional: &[],
    },
    // ── EKRecurrenceEnd ────────────────────────────────────────────────
    // init(end: Date) / init(endDate:) / init(occurrenceCount:)
    ObjectSpec {
        class_name: "EKRecurrenceEnd",
        fields: &[
            ("endDate", Default::Nil),
            ("occurrenceCount", Default::Int(0)),
        ],
        aliases: &[
            ("end", "endDate"),
            ("endDate", "endDate"),
            ("occurrenceCount", "occurrenceCount"),
        ],
        positional: &[],
    },
    // ── EKRecurrenceDayOfWeek ──────────────────────────────────────────
    // init(_:) / init(_:weekNumber:) / init(dayOfTheWeek:weekNumber:)
    ObjectSpec {
        class_name: "EKRecurrenceDayOfWeek",
        fields: &[
            ("dayOfTheWeek", Default::Enum("EKWeekday", "sunday")),
            ("weekNumber", Default::Int(0)),
        ],
        aliases: &[
            ("dayOfTheWeek", "dayOfTheWeek"),
            ("weekNumber", "weekNumber"),
        ],
        positional: &["dayOfTheWeek"],
    },
    // ── EKRecurrenceRule ───────────────────────────────────────────────
    // init(recurrenceWith:interval:end:) and the fuller day/month/year form.
    ObjectSpec {
        class_name: "EKRecurrenceRule",
        fields: &[
            ("frequency", Default::Enum("EKRecurrenceFrequency", "daily")),
            ("interval", Default::Int(1)),
            ("firstDayOfTheWeek", Default::Int(0)),
            ("daysOfTheWeek", Default::Nil),
            ("daysOfTheMonth", Default::Nil),
            ("daysOfTheYear", Default::Nil),
            ("monthsOfTheYear", Default::Nil),
            ("weeksOfTheYear", Default::Nil),
            ("setPositions", Default::Nil),
            ("recurrenceEnd", Default::Nil),
            ("calendarIdentifier", Default::Str),
        ],
        aliases: &[
            ("recurrenceWith", "frequency"),
            ("recurrenceWithFrequency", "frequency"),
            ("frequency", "frequency"),
            ("interval", "interval"),
            ("daysOfTheWeek", "daysOfTheWeek"),
            ("daysOfTheMonth", "daysOfTheMonth"),
            ("daysOfTheYear", "daysOfTheYear"),
            ("monthsOfTheYear", "monthsOfTheYear"),
            ("weeksOfTheYear", "weeksOfTheYear"),
            ("setPositions", "setPositions"),
            ("end", "recurrenceEnd"),
        ],
        positional: &[],
    },
];

/// Register every value-object constructor (already inside the `EventKit`
/// module scope). Each type's class name is interned as a builtin receiver so
/// its objects classify for member dispatch, matching core's builtin objects.
///
/// Free functions are plain `fn` pointers (no captured spec), so one thin
/// constructor per type indexes into [`SPECS`]; the shared [`build`] does the
/// work.
pub(crate) fn install(interp: &mut Interpreter<'_>) {
    for spec in SPECS {
        tswift_core::BuiltinReceiver::register_extension(spec.class_name);
    }
    interp.register_free_fn("EKAlarm", |_, args| Ok(build(&SPECS[0], args)));
    interp.register_free_fn("EKStructuredLocation", |_, args| Ok(build(&SPECS[1], args)));
    interp.register_free_fn("EKRecurrenceEnd", |_, args| Ok(build(&SPECS[2], args)));
    interp.register_free_fn("EKRecurrenceDayOfWeek", |_, args| {
        Ok(build(&SPECS[3], args))
    });
    interp.register_free_fn("EKRecurrenceRule", |_, args| Ok(build(&SPECS[4], args)));
}

/// Build a value object: seed defaults, then overlay labeled / positional args.
fn build(spec: &ObjectSpec, args: Vec<Arg>) -> SwiftValue {
    let mut fields: Vec<(String, SwiftValue)> = spec
        .fields
        .iter()
        .map(|(name, def)| ((*name).to_string(), def.seed()))
        .collect();

    let mut set_field = |field: &str, value: SwiftValue| {
        if let Some(slot) = fields.iter_mut().find(|(n, _)| n == field) {
            slot.1 = value;
        }
    };

    let mut next_positional = 0usize;
    for arg in args {
        match arg.label.as_deref() {
            Some(label) => {
                if let Some((_, field)) = spec.aliases.iter().find(|(l, _)| *l == label) {
                    set_field(field, arg.value);
                }
            }
            None => {
                if let Some(field) = spec.positional.get(next_positional) {
                    set_field(field, arg.value);
                    next_positional += 1;
                }
            }
        }
    }

    SwiftValue::Object(Rc::new(std::cell::RefCell::new(ClassObj {
        class_name: spec.class_name.to_string(),
        fields,
    })))
}

/// Coverage keys (`Type.init` + `Type.<property>`) for every value object.
pub(crate) fn coverage_keys() -> Vec<String> {
    let mut keys = Vec::new();
    for spec in SPECS {
        keys.push(format!("{}.init", spec.class_name));
        for (field, _) in spec.fields {
            keys.push(format!("{}.{}", spec.class_name, field));
        }
    }
    keys
}
