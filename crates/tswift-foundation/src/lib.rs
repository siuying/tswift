//! tswift-foundation — native Foundation value builtins.
//!
//! The crate mirrors the `tswift-std` registry seam: install once into an
//! interpreter, expose live `registered_keys()` for coverage tooling, and keep
//! behaviour slices small enough to validate with CLI golden fixtures.

mod calendar;
mod decimal;
mod formatter;
mod measurement;
mod numberformatter;
mod url;

use std::{collections::BTreeSet, rc::Rc};

use tswift_core::{
    Arg, BuiltinReceiver, EnumObj, EvalError, Interpreter, IntrinsicFn, MethodEntry, Outcome,
    StdContext, StdError, StdResult, StructObj, SwiftValue,
};

const REFERENCE_DATE_UNIX_OFFSET: f64 = 978_307_200.0;
const DISTANT_PAST_REFERENCE_SECONDS: f64 = -63_113_904_000.0;
const DISTANT_FUTURE_REFERENCE_SECONDS: f64 = 63_113_904_000.0;

/// Register every currently-supported Foundation builtin into `interp`.
pub fn install(interp: &mut Interpreter<'_>) {
    url::install(interp);
    calendar::install(interp);
    formatter::install(interp);
    decimal::install(interp);
    numberformatter::install(interp);
    measurement::install(interp);
    interp.register_free_fn("Date", date_init);
    interp.register_property(
        BuiltinReceiver::Date,
        "timeIntervalSinceReferenceDate",
        date_time_interval_since_reference_date,
    );
    interp.register_property(
        BuiltinReceiver::Date,
        "timeIntervalSince1970",
        date_time_interval_since_1970,
    );
    interp.register_contextual_property(
        BuiltinReceiver::Date,
        "timeIntervalSinceNow",
        date_time_interval_since_now,
    );
    interp.register_static(BuiltinReceiver::Date, "now", date_now_static);
    interp.register_static(BuiltinReceiver::Date, "distantPast", date_distant_past);
    interp.register_static(BuiltinReceiver::Date, "distantFuture", date_distant_future);
    interp.register_static(
        BuiltinReceiver::Date,
        "timeIntervalBetween1970AndReferenceDate",
        date_time_interval_between_1970_and_reference_date,
    );
    for (name, mutating, func) in [
        (
            "timeIntervalSince",
            false,
            date_time_interval_since as IntrinsicFn,
        ),
        ("addingTimeInterval", false, date_adding_time_interval),
        ("addTimeInterval", true, date_add_time_interval),
        ("distance", false, date_distance),
        ("advanced", false, date_advanced),
        ("compare", false, date_compare),
    ] {
        interp.register_intrinsic(BuiltinReceiver::Date, name, MethodEntry { mutating, func });
    }
    interp.register_property(BuiltinReceiver::Date, "description", date_description);
    interp.register_property(BuiltinReceiver::Date, "debugDescription", date_description);
    interp.register_property(BuiltinReceiver::Date, "hashValue", date_hash_value);

    interp.register_free_fn("DateComponents", date_components_init);
    for (name, getter) in DATE_COMPONENT_GETTERS {
        interp.register_property(BuiltinReceiver::DateComponents, name, *getter);
    }
    interp.register_property(
        BuiltinReceiver::DateComponents,
        "isValidDate",
        date_components_is_valid_date,
    );
    for (name, mutating, func) in [
        ("value", false, date_components_value as IntrinsicFn),
        ("setValue", true, date_components_set_value),
    ] {
        interp.register_intrinsic(
            BuiltinReceiver::DateComponents,
            name,
            MethodEntry { mutating, func },
        );
    }

    interp.register_free_fn("Data", data_init);
    interp.register_property(BuiltinReceiver::Data, "count", data_count);
    interp.register_property(BuiltinReceiver::Data, "isEmpty", data_is_empty);
    interp.register_property(BuiltinReceiver::Data, "first", data_first);
    interp.register_property(BuiltinReceiver::Data, "last", data_last);
    interp.register_property(BuiltinReceiver::Data, "description", data_description);
    for (name, mutating, func) in [
        ("append", true, data_append as IntrinsicFn),
        ("base64EncodedString", false, data_base64_encoded_string),
        ("subdata", false, data_subdata),
        ("removeAll", true, data_remove_all),
    ] {
        interp.register_intrinsic(BuiltinReceiver::Data, name, MethodEntry { mutating, func });
    }

    interp.register_free_fn("UUID", uuid_init);
    interp.register_property(BuiltinReceiver::UUID, "uuidString", uuid_string);
    interp.register_property(BuiltinReceiver::UUID, "description", uuid_description);

    interp.register_free_fn("IndexPath", index_path_init);
    interp.register_property(BuiltinReceiver::IndexPath, "count", index_path_count);
    interp.register_property(BuiltinReceiver::IndexPath, "isEmpty", index_path_is_empty);
    interp.register_intrinsic(
        BuiltinReceiver::IndexPath,
        "append",
        MethodEntry {
            mutating: true,
            func: index_path_append,
        },
    );
    interp.register_intrinsic(
        BuiltinReceiver::IndexPath,
        "appending",
        MethodEntry {
            mutating: false,
            func: index_path_appending,
        },
    );
    interp.register_property(
        BuiltinReceiver::IndexPath,
        "startIndex",
        index_path_start_index,
    );
    interp.register_property(BuiltinReceiver::IndexPath, "endIndex", index_path_end_index);
    interp.register_intrinsic(
        BuiltinReceiver::IndexPath,
        "dropLast",
        MethodEntry {
            mutating: false,
            func: index_path_drop_last,
        },
    );

    interp.register_free_fn("IndexSet", index_set_init);
    interp.register_property(BuiltinReceiver::IndexSet, "count", index_set_count);
    interp.register_property(BuiltinReceiver::IndexSet, "isEmpty", index_set_is_empty);
    interp.register_property(BuiltinReceiver::IndexSet, "first", index_set_first);
    interp.register_property(BuiltinReceiver::IndexSet, "last", index_set_last);
    interp.register_intrinsic(
        BuiltinReceiver::IndexSet,
        "contains",
        MethodEntry {
            mutating: false,
            func: index_set_contains,
        },
    );
    interp.register_intrinsic(
        BuiltinReceiver::IndexSet,
        "insert",
        MethodEntry {
            mutating: true,
            func: index_set_insert,
        },
    );
    interp.register_intrinsic(
        BuiltinReceiver::IndexSet,
        "remove",
        MethodEntry {
            mutating: true,
            func: index_set_remove,
        },
    );
    interp.register_intrinsic(
        BuiltinReceiver::IndexSet,
        "removeAll",
        MethodEntry {
            mutating: true,
            func: index_set_remove_all,
        },
    );
    interp.register_intrinsic(
        BuiltinReceiver::IndexSet,
        "update",
        MethodEntry {
            mutating: true,
            func: index_set_update,
        },
    );
    let non_mutating: [(&str, IntrinsicFn); 7] = [
        ("union", index_set_union),
        ("intersection", index_set_intersection),
        ("symmetricDifference", index_set_symmetric_difference),
        ("integerGreaterThan", index_set_integer_greater_than),
        ("integerLessThan", index_set_integer_less_than),
        ("integerGreaterThanOrEqualTo", index_set_integer_ge),
        ("integerLessThanOrEqualTo", index_set_integer_le),
    ];
    for (name, func) in non_mutating {
        interp.register_intrinsic(
            BuiltinReceiver::IndexSet,
            name,
            MethodEntry {
                mutating: false,
                func,
            },
        );
    }
    let mutating: [(&str, IntrinsicFn); 3] = [
        ("formUnion", index_set_form_union),
        ("formIntersection", index_set_form_intersection),
        (
            "formSymmetricDifference",
            index_set_form_symmetric_difference,
        ),
    ];
    for (name, func) in mutating {
        interp.register_intrinsic(
            BuiltinReceiver::IndexSet,
            name,
            MethodEntry {
                mutating: true,
                func,
            },
        );
    }
}

/// Every Foundation entry registered by [`install`], as coverage keys.
pub fn registered_keys() -> Vec<String> {
    let mut sink = std::io::sink();
    let mut interp = Interpreter::new(&mut sink);
    install(&mut interp);
    let mut keys: Vec<String> = interp
        .registered_keys()
        .into_iter()
        .filter_map(|key| match key.as_str() {
            "Data" => Some("Data.init".to_string()),
            "UUID" => Some("UUID.init".to_string()),
            "IndexPath" => Some("IndexPath.init".to_string()),
            "IndexSet" => Some("IndexSet.init".to_string()),
            "URL" => Some("URL.init".to_string()),
            "URLComponents" => Some("URLComponents.init".to_string()),
            "URLQueryItem" => Some("URLQueryItem.init".to_string()),
            "Date" => Some("Date.init".to_string()),
            "DateComponents" => Some("DateComponents.init".to_string()),
            "Calendar" => Some("Calendar.init".to_string()),
            "DateFormatter" => Some("DateFormatter.init".to_string()),
            "ISO8601DateFormatter" => Some("ISO8601DateFormatter.init".to_string()),
            "Decimal" => Some("Decimal.init".to_string()),
            "NumberFormatter" => Some("NumberFormatter.init".to_string()),
            "Measurement" => Some("Measurement.init".to_string()),
            other
                if other.starts_with("Data.")
                    || other.starts_with("UUID.")
                    || other.starts_with("IndexPath.")
                    || other.starts_with("IndexSet.")
                    || other.starts_with("URL.")
                    || other.starts_with("URLComponents.")
                    || other.starts_with("URLQueryItem.")
                    || other.starts_with("Date.")
                    || other.starts_with("DateComponents.")
                    || other.starts_with("Calendar.")
                    || other.starts_with("DateFormatter.")
                    || other.starts_with("ISO8601DateFormatter.")
                    || other.starts_with("Decimal.")
                    || other.starts_with("NumberFormatter.")
                    || other.starts_with("Measurement.") =>
            {
                Some(other.to_string())
            }
            _ => None,
        })
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

pub(crate) fn type_error(message: impl Into<String>) -> StdError {
    StdError::Error(EvalError::Type(message.into()))
}

pub(crate) fn date_value(time_interval_since_reference_date: f64) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "Date".into(),
        fields: vec![(
            "_timeIntervalSinceReferenceDate".into(),
            SwiftValue::Double(time_interval_since_reference_date),
        )],
    }))
}

pub(crate) fn date_seconds(value: &SwiftValue) -> Result<f64, StdError> {
    let SwiftValue::Struct(obj) = value else {
        return Err(type_error(format!(
            "expected Date, got {}",
            value.type_name()
        )));
    };
    if obj.type_name != "Date" {
        return Err(type_error(format!("expected Date, got {}", obj.type_name)));
    }
    match obj.get("_timeIntervalSinceReferenceDate") {
        Some(SwiftValue::Double(seconds)) => Ok(*seconds),
        Some(SwiftValue::Int(seconds)) => Ok(seconds.raw as f64),
        _ => Err(type_error("malformed Date value")),
    }
}

fn time_interval(value: &SwiftValue, context: &str) -> Result<f64, StdError> {
    match value {
        SwiftValue::Double(seconds) => Ok(*seconds),
        SwiftValue::Int(seconds) => Ok(seconds.raw as f64),
        other => Err(type_error(format!(
            "{context} expects TimeInterval, got {}",
            other.type_name()
        ))),
    }
}

fn date_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    match args.as_slice() {
        [] => Ok(date_value(
            ctx.now_unix_seconds() - REFERENCE_DATE_UNIX_OFFSET,
        )),
        [arg] if arg.label.as_deref() == Some("timeIntervalSinceReferenceDate") => Ok(date_value(
            time_interval(&arg.value, "Date(timeIntervalSinceReferenceDate:)")?,
        )),
        [arg] if arg.label.as_deref() == Some("timeIntervalSince1970") => Ok(date_value(
            time_interval(&arg.value, "Date(timeIntervalSince1970:)")? - REFERENCE_DATE_UNIX_OFFSET,
        )),
        [arg] if arg.label.as_deref() == Some("timeIntervalSinceNow") => Ok(date_value(
            ctx.now_unix_seconds() - REFERENCE_DATE_UNIX_OFFSET
                + time_interval(&arg.value, "Date(timeIntervalSinceNow:)")?,
        )),
        [interval, since]
            if interval.label.as_deref() == Some("timeInterval")
                && since.label.as_deref() == Some("since") =>
        {
            Ok(date_value(
                date_seconds(&since.value)?
                    + time_interval(&interval.value, "Date(timeInterval:since:)")?,
            ))
        }
        _ => Err(type_error("unsupported Date initializer arguments")),
    }
}

/// `Date.description`: ISO-like UTC instant `"yyyy-MM-dd HH:mm:ss +0000"`, the
/// Darwin default. `debugDescription` is identical.
fn date_description(recv: SwiftValue) -> StdResult {
    let civil = crate::calendar::decompose(date_seconds(&recv)?);
    Ok(SwiftValue::Str(
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02} +0000",
            civil.year, civil.month, civil.day, civil.hour, civil.minute, civil.second
        )
        .into(),
    ))
}

/// `Date.hashValue`: hash of the reference-date offset. Equal dates compare
/// equal on that Double, so this stays consistent with `==`. `+ 0.0`
/// canonicalizes `-0.0` to `0.0` (which `==` treats as equal), and the bit
/// pattern is narrowed through `i64` to stay within the platform `Int`.
fn date_hash_value(recv: SwiftValue) -> StdResult {
    let seconds = date_seconds(&recv)? + 0.0;
    Ok(SwiftValue::int((seconds.to_bits() as i64) as i128))
}

fn date_time_interval_since_reference_date(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Double(date_seconds(&recv)?))
}

fn date_time_interval_since_1970(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Double(
        date_seconds(&recv)? + REFERENCE_DATE_UNIX_OFFSET,
    ))
}

fn date_time_interval_since_now(ctx: &mut dyn StdContext, recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Double(
        date_seconds(&recv)? + REFERENCE_DATE_UNIX_OFFSET - ctx.now_unix_seconds(),
    ))
}

fn date_now_static(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if !args.is_empty() {
        return Err(type_error("Date.now takes no arguments"));
    }
    Ok(date_value(
        ctx.now_unix_seconds() - REFERENCE_DATE_UNIX_OFFSET,
    ))
}

fn date_distant_past(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if !args.is_empty() {
        return Err(type_error("Date.distantPast takes no arguments"));
    }
    Ok(date_value(DISTANT_PAST_REFERENCE_SECONDS))
}

fn date_distant_future(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if !args.is_empty() {
        return Err(type_error("Date.distantFuture takes no arguments"));
    }
    Ok(date_value(DISTANT_FUTURE_REFERENCE_SECONDS))
}

fn date_time_interval_between_1970_and_reference_date(
    _ctx: &mut dyn StdContext,
    args: Vec<Arg>,
) -> StdResult {
    if !args.is_empty() {
        return Err(type_error(
            "Date.timeIntervalBetween1970AndReferenceDate takes no arguments",
        ));
    }
    Ok(SwiftValue::Double(REFERENCE_DATE_UNIX_OFFSET))
}

fn date_single_time_interval_arg(args: &[SwiftValue], context: &str) -> Result<f64, StdError> {
    match args {
        [value] => time_interval(value, context),
        _ => Err(type_error(format!("{context} expects one argument"))),
    }
}

fn date_single_date_arg(args: &[SwiftValue], context: &str) -> Result<f64, StdError> {
    match args {
        [value] => date_seconds(value),
        _ => Err(type_error(format!("{context} expects one Date argument"))),
    }
}

fn date_time_interval_since(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let other = date_single_date_arg(&args, "Date.timeIntervalSince")?;
    Ok(Outcome {
        result: SwiftValue::Double(date_seconds(&recv)? - other),
        receiver: recv,
    })
}

fn date_adding_time_interval(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let interval = date_single_time_interval_arg(&args, "Date.addingTimeInterval")?;
    Ok(Outcome {
        result: date_value(date_seconds(&recv)? + interval),
        receiver: recv,
    })
}

fn date_add_time_interval(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let interval = date_single_time_interval_arg(&args, "Date.addTimeInterval")?;
    let receiver = date_value(date_seconds(&recv)? + interval);
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver,
    })
}

fn date_distance(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let other = date_single_date_arg(&args, "Date.distance")?;
    Ok(Outcome {
        result: SwiftValue::Double(other - date_seconds(&recv)?),
        receiver: recv,
    })
}

fn date_advanced(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let interval = date_single_time_interval_arg(&args, "Date.advanced")?;
    Ok(Outcome {
        result: date_value(date_seconds(&recv)? + interval),
        receiver: recv,
    })
}

fn comparison_result(case: &str) -> SwiftValue {
    SwiftValue::Enum(Rc::new(EnumObj {
        type_name: "ComparisonResult".into(),
        case: case.into(),
        payload: Vec::new(),
    }))
}

fn date_compare(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let other = date_single_date_arg(&args, "Date.compare")?;
    let this = date_seconds(&recv)?;
    let case = if this < other {
        "orderedAscending"
    } else if this > other {
        "orderedDescending"
    } else {
        "orderedSame"
    };
    Ok(Outcome {
        result: comparison_result(case),
        receiver: recv,
    })
}

// ---------------------------------------------------------------------------
// DateComponents
// ---------------------------------------------------------------------------

/// The component fields stored on a `DateComponents` value, in canonical order.
pub(crate) const DATE_COMPONENT_FIELDS: &[&str] = &[
    "year",
    "month",
    "day",
    "hour",
    "minute",
    "second",
    "nanosecond",
    "weekday",
    "weekdayOrdinal",
    "quarter",
    "weekOfMonth",
    "weekOfYear",
    "yearForWeekOfYear",
    "era",
    "dayOfYear",
];

/// Initializer labels that are accepted but not stored as readable components.
const DATE_COMPONENT_IGNORED_LABELS: &[&str] = &["calendar", "timeZone"];

macro_rules! date_component_getters {
    ($($field:literal => $getter:ident),+ $(,)?) => {
        $(
            fn $getter(recv: SwiftValue) -> StdResult {
                date_components_get(&recv, $field)
            }
        )+
        const DATE_COMPONENT_GETTERS: &[(&str, tswift_core::PropertyFn)] = &[
            $(($field, $getter)),+
        ];
    };
}

date_component_getters! {
    "year" => date_components_get_year,
    "month" => date_components_get_month,
    "day" => date_components_get_day,
    "hour" => date_components_get_hour,
    "minute" => date_components_get_minute,
    "second" => date_components_get_second,
    "nanosecond" => date_components_get_nanosecond,
    "weekday" => date_components_get_weekday,
    "weekdayOrdinal" => date_components_get_weekday_ordinal,
    "quarter" => date_components_get_quarter,
    "weekOfMonth" => date_components_get_week_of_month,
    "weekOfYear" => date_components_get_week_of_year,
    "yearForWeekOfYear" => date_components_get_year_for_week_of_year,
    "era" => date_components_get_era,
    "dayOfYear" => date_components_get_day_of_year,
}

pub(crate) fn date_components_value_struct(fields: Vec<(String, SwiftValue)>) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "DateComponents".into(),
        fields,
    }))
}

fn date_components_obj(value: &SwiftValue) -> Result<&Rc<StructObj>, StdError> {
    match value {
        SwiftValue::Struct(obj) if obj.type_name == "DateComponents" => Ok(obj),
        other => Err(type_error(format!(
            "expected DateComponents, got {}",
            other.type_name()
        ))),
    }
}

/// Parse an optional Int component argument (`Int?`): `nil` or an integer.
fn optional_int_component(value: &SwiftValue, context: &str) -> Result<SwiftValue, StdError> {
    match value {
        SwiftValue::Nil => Ok(SwiftValue::Nil),
        SwiftValue::Int(i) => Ok(SwiftValue::int(i.raw)),
        other => Err(type_error(format!(
            "{context} expects Int?, got {}",
            other.type_name()
        ))),
    }
}

fn date_components_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut fields: Vec<(String, SwiftValue)> = DATE_COMPONENT_FIELDS
        .iter()
        .map(|name| ((*name).to_string(), SwiftValue::Nil))
        .collect();
    for arg in &args {
        let Some(label) = arg.label.as_deref() else {
            return Err(type_error(
                "DateComponents initializer requires labeled arguments",
            ));
        };
        if DATE_COMPONENT_IGNORED_LABELS.contains(&label) {
            continue;
        }
        let Some(slot) = fields.iter_mut().find(|(name, _)| name == label) else {
            return Err(type_error(format!(
                "DateComponents has no component `{label}`"
            )));
        };
        slot.1 = optional_int_component(&arg.value, &format!("DateComponents(.{label}:)"))?;
    }
    Ok(date_components_value_struct(fields))
}

fn date_components_get(value: &SwiftValue, component: &str) -> StdResult {
    let obj = date_components_obj(value)?;
    Ok(obj.get(component).cloned().unwrap_or(SwiftValue::Nil))
}

/// Component name behind a `Calendar.Component` enum value (`.year` → "year").
pub(crate) fn calendar_component_name(value: &SwiftValue) -> Result<String, StdError> {
    match value {
        SwiftValue::Enum(obj) => Ok(obj.case.clone()),
        SwiftValue::Str(name) => Ok(name.to_string()),
        other => Err(type_error(format!(
            "expected Calendar.Component, got {}",
            other.type_name()
        ))),
    }
}

fn date_components_value(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let [component] = args.as_slice() else {
        return Err(type_error(
            "DateComponents.value(for:) expects one argument",
        ));
    };
    let name = calendar_component_name(component)?;
    let result = if DATE_COMPONENT_FIELDS.contains(&name.as_str()) {
        date_components_obj(&recv)?
            .get(&name)
            .cloned()
            .unwrap_or(SwiftValue::Nil)
    } else {
        SwiftValue::Nil
    };
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

fn date_components_set_value(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let [value, component] = args.as_slice() else {
        return Err(type_error(
            "DateComponents.setValue(_:for:) expects two arguments",
        ));
    };
    let name = calendar_component_name(component)?;
    let obj = date_components_obj(&recv)?;
    let mut fields = obj.fields.clone();
    if let Some(slot) = fields.iter_mut().find(|(field, _)| field == &name) {
        slot.1 = optional_int_component(value, "DateComponents.setValue(_:for:)")?;
    }
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: date_components_value_struct(fields),
    })
}

/// Minimal range validation; full calendar validation arrives with Calendar.
fn date_components_is_valid_date(recv: SwiftValue) -> StdResult {
    let obj = date_components_obj(&recv)?;
    let component = |name: &str| match obj.get(name) {
        Some(SwiftValue::Int(i)) => Some(i.raw),
        _ => None,
    };
    let valid = match (component("year"), component("month"), component("day")) {
        (Some(year), Some(month), Some(day)) => {
            (1..=12).contains(&month)
                && day >= 1
                && day <= i128::from(calendar::days_in_month(year as i64, month as i64))
        }
        _ => false,
    };
    Ok(SwiftValue::Bool(valid))
}

fn data_value(bytes: Vec<u8>) -> SwiftValue {
    let elements = bytes
        .into_iter()
        .map(|b| SwiftValue::int(i128::from(b)))
        .collect();
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "Data".into(),
        fields: vec![("_bytes".into(), SwiftValue::Array(Rc::new(elements)))],
    }))
}

fn data_bytes(value: &SwiftValue) -> Result<Vec<u8>, StdError> {
    let SwiftValue::Struct(obj) = value else {
        return Err(type_error(format!(
            "expected Data, got {}",
            value.type_name()
        )));
    };
    if obj.type_name != "Data" {
        return Err(type_error(format!("expected Data, got {}", obj.type_name)));
    }
    let Some(SwiftValue::Array(items)) = obj.get("_bytes") else {
        return Err(type_error("malformed Data value"));
    };
    items
        .iter()
        .map(byte_from_value)
        .collect::<Result<Vec<_>, _>>()
}

fn byte_from_value(value: &SwiftValue) -> Result<u8, StdError> {
    match value {
        SwiftValue::Int(i) if (0..=255).contains(&i.raw) => Ok(i.raw as u8),
        SwiftValue::Int(i) => Err(type_error(format!("byte value {} out of range", i.raw))),
        other => Err(type_error(format!(
            "expected UInt8 byte, got {}",
            other.type_name()
        ))),
    }
}

fn data_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if args.is_empty() {
        return Ok(data_value(Vec::new()));
    }
    // `Data(repeating: UInt8, count: Int)`.
    if args.len() == 2
        && args[0].label.as_deref() == Some("repeating")
        && args[1].label.as_deref() == Some("count")
    {
        let byte = byte_from_value(&args[0].value)?;
        let count = match &args[1].value {
            SwiftValue::Int(i) if i.raw >= 0 => i.raw as usize,
            _ => {
                return Err(type_error(
                    "Data(repeating:count:) count must be a non-negative Int",
                ))
            }
        };
        return Ok(data_value(vec![byte; count]));
    }
    if args.len() != 1 {
        return Err(type_error(
            "Data expects zero arguments or one byte sequence",
        ));
    }
    if args.len() == 1 && args[0].label.as_deref() == Some("base64Encoded") {
        let SwiftValue::Str(s) = &args[0].value else {
            return Err(type_error("Data(base64Encoded:) expects a String"));
        };
        // Failable: nil on malformed input.
        return Ok(match base64_decode(s) {
            Some(bytes) => data_value(bytes),
            None => SwiftValue::Nil,
        });
    }
    match args[0].label.as_deref() {
        Some("bytes") | None => {}
        Some(other) => {
            return Err(type_error(format!(
                "unsupported Data initializer `{other}:`"
            )))
        }
    }
    match &args[0].value {
        SwiftValue::Array(items) => {
            let bytes = items
                .iter()
                .map(byte_from_value)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(data_value(bytes))
        }
        SwiftValue::Struct(obj) if obj.type_name == "Data" => Ok(args[0].value.clone()),
        other => Err(type_error(format!(
            "Data expects [UInt8] or Data, got {}",
            other.type_name()
        ))),
    }
}

fn data_first(recv: SwiftValue) -> StdResult {
    let bytes = data_bytes(&recv)?;
    Ok(bytes
        .first()
        .map(|b| SwiftValue::int(i128::from(*b)))
        .unwrap_or(SwiftValue::Nil))
}

fn data_last(recv: SwiftValue) -> StdResult {
    let bytes = data_bytes(&recv)?;
    Ok(bytes
        .last()
        .map(|b| SwiftValue::int(i128::from(*b)))
        .unwrap_or(SwiftValue::Nil))
}

fn data_description(recv: SwiftValue) -> StdResult {
    let len = data_bytes(&recv)?.len();
    // Foundation renders e.g. "5 bytes".
    let unit = if len == 1 { "byte" } else { "bytes" };
    Ok(SwiftValue::Str(format!("{len} {unit}").into()))
}

fn data_base64_encoded_string(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !args.is_empty() {
        return Err(type_error("base64EncodedString() takes no arguments"));
    }
    let encoded = base64_encode(&data_bytes(&recv)?);
    Ok(Outcome {
        result: SwiftValue::Str(encoded.into()),
        receiver: recv,
    })
}

fn data_subdata(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let bytes = data_bytes(&recv)?;
    let range = match args.as_slice() {
        [SwiftValue::Range { lo, hi, inclusive }] => {
            let end = if *inclusive { hi + 1 } else { *hi };
            (*lo, end)
        }
        _ => return Err(type_error("subdata(in:) expects a Range<Int>")),
    };
    let (lo, hi) = range;
    if lo < 0 || hi < lo || hi as usize > bytes.len() {
        return Err(type_error("subdata(in:) range out of bounds"));
    }
    let slice = bytes[lo as usize..hi as usize].to_vec();
    Ok(Outcome {
        result: data_value(slice),
        receiver: recv,
    })
}

fn data_remove_all(
    _ctx: &mut dyn StdContext,
    _recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !args.is_empty() {
        return Err(type_error("removeAll() takes no arguments"));
    }
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: data_value(Vec::new()),
    })
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(BASE64_ALPHABET[(triple >> 18 & 0x3F) as usize] as char);
        out.push(BASE64_ALPHABET[(triple >> 12 & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            BASE64_ALPHABET[(triple >> 6 & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            BASE64_ALPHABET[(triple & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let cleaned: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    // Empty input decodes to empty Data (matching Foundation); other lengths
    // must be a whole number of 4-char groups.
    if cleaned.len() % 4 != 0 {
        return None;
    }
    let chunk_count = cleaned.len() / 4;
    let mut out = Vec::with_capacity(chunk_count * 3);
    for (chunk_index, chunk) in cleaned.chunks(4).enumerate() {
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        // Padding is only ever valid (1 or 2 chars) in the final chunk, and the
        // pad must be a trailing run.
        if pad > 0 && (chunk_index != chunk_count - 1 || pad > 2) {
            return None;
        }
        if pad > 0 && chunk[4 - pad..].iter().any(|&c| c != b'=') {
            return None;
        }
        let mut acc = 0u32;
        for &c in &chunk[..4 - pad] {
            acc = (acc << 6) | val(c)?;
        }
        acc <<= 6 * pad;
        out.push((acc >> 16 & 0xFF) as u8);
        if pad < 2 {
            out.push((acc >> 8 & 0xFF) as u8);
        }
        if pad < 1 {
            out.push((acc & 0xFF) as u8);
        }
    }
    Some(out)
}

fn data_count(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(data_bytes(&recv)?.len() as i128))
}

fn data_is_empty(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(data_bytes(&recv)?.is_empty()))
}

fn data_append(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let mut bytes = data_bytes(&recv)?;
    for arg in args {
        match &arg {
            SwiftValue::Struct(obj) if obj.type_name == "Data" => bytes.extend(data_bytes(&arg)?),
            _ => bytes.push(byte_from_value(&arg)?),
        }
    }
    let receiver = data_value(bytes);
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver,
    })
}

fn uuid_value(uuid: String) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "UUID".into(),
        fields: vec![("uuidString".into(), SwiftValue::Str(uuid))],
    }))
}

fn uuid_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if args.is_empty() {
        return Ok(uuid_value(random_uuid(ctx)));
    }
    if args.len() == 1 && args[0].label.as_deref() == Some("uuidString") {
        let SwiftValue::Str(raw) = &args[0].value else {
            return Err(type_error("UUID(uuidString:) expects String"));
        };
        return Ok(match normalize_uuid(raw) {
            Some(uuid) => uuid_value(uuid),
            None => SwiftValue::Nil,
        });
    }
    Err(type_error("UUID expects no arguments or uuidString:"))
}

fn uuid_string(recv: SwiftValue) -> StdResult {
    let SwiftValue::Struct(obj) = recv else {
        return Err(type_error("uuidString expects UUID"));
    };
    if obj.type_name != "UUID" {
        return Err(type_error("uuidString expects UUID"));
    }
    match obj.get("uuidString") {
        Some(SwiftValue::Str(s)) => Ok(SwiftValue::Str(s.clone())),
        _ => Err(type_error("malformed UUID value")),
    }
}

fn uuid_description(recv: SwiftValue) -> StdResult {
    // `UUID.description` is its canonical uppercase string.
    uuid_string(recv)
}

fn normalize_uuid(raw: &str) -> Option<String> {
    let upper = raw.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    let groups = [8, 13, 18, 23];
    if bytes.len() != 36 || groups.iter().any(|&i| bytes[i] != b'-') {
        return None;
    }
    if bytes
        .iter()
        .enumerate()
        .any(|(i, b)| !groups.contains(&i) && !b.is_ascii_hexdigit())
    {
        return None;
    }
    Some(upper)
}

fn random_uuid(ctx: &mut dyn StdContext) -> String {
    let mut bytes = [0u8; 16];
    for chunk in bytes.chunks_mut(8) {
        let rand = ctx.random_u64().to_be_bytes();
        chunk.copy_from_slice(&rand[..chunk.len()]);
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

fn int_arg(value: &SwiftValue, context: &str) -> Result<i128, StdError> {
    match value {
        SwiftValue::Int(i) => Ok(i.raw),
        other => Err(type_error(format!(
            "{context} expects Int, got {}",
            other.type_name()
        ))),
    }
}

fn int_array_arg(value: &SwiftValue, context: &str) -> Result<Vec<i128>, StdError> {
    match value {
        SwiftValue::Array(items) => items
            .iter()
            .map(|item| int_arg(item, context))
            .collect::<Result<Vec<_>, _>>(),
        other => Err(type_error(format!(
            "{context} expects [Int], got {}",
            other.type_name()
        ))),
    }
}

fn index_path_value(indexes: Vec<i128>) -> SwiftValue {
    let items = indexes.into_iter().map(SwiftValue::int).collect();
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "IndexPath".into(),
        fields: vec![("_indexes".into(), SwiftValue::Array(Rc::new(items)))],
    }))
}

fn index_path_indexes(value: &SwiftValue) -> Result<Vec<i128>, StdError> {
    let SwiftValue::Struct(obj) = value else {
        return Err(type_error(format!(
            "expected IndexPath, got {}",
            value.type_name()
        )));
    };
    if obj.type_name != "IndexPath" {
        return Err(type_error(format!(
            "expected IndexPath, got {}",
            obj.type_name
        )));
    }
    let Some(indexes) = obj.get("_indexes") else {
        return Err(type_error("malformed IndexPath value"));
    };
    int_array_arg(indexes, "IndexPath")
}

fn index_path_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if args.is_empty() {
        return Ok(index_path_value(Vec::new()));
    }
    if args.len() != 1 {
        return Err(type_error("IndexPath expects zero or one argument"));
    }
    match args[0].label.as_deref() {
        Some("indexes") => Ok(index_path_value(int_array_arg(
            &args[0].value,
            "IndexPath(indexes:) ",
        )?)),
        Some("index") => Ok(index_path_value(vec![int_arg(
            &args[0].value,
            "IndexPath(index:) ",
        )?])),
        Some(label) => Err(type_error(format!(
            "unsupported IndexPath argument {label}:"
        ))),
        None => Err(type_error("IndexPath argument needs a label")),
    }
}

fn index_path_count(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(index_path_indexes(&recv)?.len() as i128))
}

fn index_path_is_empty(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(index_path_indexes(&recv)?.is_empty()))
}

/// `startIndex` — always `0` (an `IndexPath` is a zero-based collection).
fn index_path_start_index(_recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(0))
}

/// `endIndex` — the past-the-end position, i.e. the element count.
fn index_path_end_index(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(index_path_indexes(&recv)?.len() as i128))
}

/// `dropLast(_:)` — a new `IndexPath` without its last `k` (default 1) indexes.
fn index_path_drop_last(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let k = match args.as_slice() {
        [] => 1,
        [n] => {
            let n = int_arg(n, "IndexPath.dropLast")?;
            if n < 0 {
                return Err(type_error(
                    "IndexPath.dropLast: can't drop a negative number of elements",
                ));
            }
            n as usize
        }
        _ => return Err(type_error("IndexPath.dropLast expects zero or one Int")),
    };
    let mut indexes = index_path_indexes(&recv)?;
    let keep = indexes.len().saturating_sub(k);
    indexes.truncate(keep);
    Ok(Outcome {
        result: index_path_value(indexes),
        receiver: recv,
    })
}

fn index_path_append(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if args.len() != 1 {
        return Err(type_error("IndexPath.append expects one argument"));
    }
    let mut indexes = index_path_indexes(&recv)?;
    for arg in args {
        match arg {
            SwiftValue::Struct(obj) if obj.type_name == "IndexPath" => {
                indexes.extend(index_path_indexes(&SwiftValue::Struct(obj))?);
            }
            SwiftValue::Array(_) => indexes.extend(int_array_arg(&arg, "IndexPath.append")?),
            _ => indexes.push(int_arg(&arg, "IndexPath.append")?),
        }
    }
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: index_path_value(indexes),
    })
}

fn index_path_appending(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if args.len() != 1 {
        return Err(type_error("IndexPath.appending expects one argument"));
    }
    let mut indexes = index_path_indexes(&recv)?;
    for arg in args {
        match arg {
            SwiftValue::Struct(obj) if obj.type_name == "IndexPath" => {
                indexes.extend(index_path_indexes(&SwiftValue::Struct(obj))?);
            }
            SwiftValue::Array(_) => indexes.extend(int_array_arg(&arg, "IndexPath.appending")?),
            _ => indexes.push(int_arg(&arg, "IndexPath.appending")?),
        }
    }
    Ok(Outcome {
        result: index_path_value(indexes),
        receiver: recv,
    })
}

fn index_set_value(values: BTreeSet<i128>) -> SwiftValue {
    let items = values.into_iter().map(SwiftValue::int).collect();
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "IndexSet".into(),
        fields: vec![("_values".into(), SwiftValue::Array(Rc::new(items)))],
    }))
}

fn index_set_values(value: &SwiftValue) -> Result<BTreeSet<i128>, StdError> {
    let SwiftValue::Struct(obj) = value else {
        return Err(type_error(format!(
            "expected IndexSet, got {}",
            value.type_name()
        )));
    };
    if obj.type_name != "IndexSet" {
        return Err(type_error(format!(
            "expected IndexSet, got {}",
            obj.type_name
        )));
    }
    let Some(values) = obj.get("_values") else {
        return Err(type_error("malformed IndexSet value"));
    };
    Ok(int_array_arg(values, "IndexSet")?.into_iter().collect())
}

fn ints_in_range(value: &SwiftValue, context: &str) -> Result<Vec<i128>, StdError> {
    match value {
        SwiftValue::Range { lo, hi, inclusive } => {
            let end = if *inclusive {
                hi.saturating_add(1)
            } else {
                *hi
            };
            Ok((*lo..end).collect())
        }
        other => Err(type_error(format!(
            "{context} expects Range<Int>, got {}",
            other.type_name()
        ))),
    }
}

fn index_set_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if args.is_empty() {
        return Ok(index_set_value(BTreeSet::new()));
    }
    if args.len() != 1 {
        return Err(type_error("IndexSet expects zero or one argument"));
    }
    let values = match args[0].label.as_deref() {
        Some("integer") => [int_arg(&args[0].value, "IndexSet(integer:) ")?]
            .into_iter()
            .collect(),
        Some("integersIn") => ints_in_range(&args[0].value, "IndexSet(integersIn:) ")?
            .into_iter()
            .collect(),
        Some(label) => {
            return Err(type_error(format!(
                "unsupported IndexSet argument {label}:"
            )))
        }
        None => return Err(type_error("IndexSet argument needs a label")),
    };
    Ok(index_set_value(values))
}

fn index_set_count(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(index_set_values(&recv)?.len() as i128))
}

fn index_set_is_empty(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(index_set_values(&recv)?.is_empty()))
}

fn index_set_first(recv: SwiftValue) -> StdResult {
    Ok(index_set_values(&recv)?
        .first()
        .copied()
        .map(SwiftValue::int)
        .unwrap_or(SwiftValue::Nil))
}

fn index_set_last(recv: SwiftValue) -> StdResult {
    Ok(index_set_values(&recv)?
        .last()
        .copied()
        .map(SwiftValue::int)
        .unwrap_or(SwiftValue::Nil))
}

fn index_set_contains(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if args.len() != 1 {
        return Err(type_error("IndexSet.contains expects one Int"));
    }
    let value = int_arg(&args[0], "IndexSet.contains")?;
    Ok(Outcome {
        result: SwiftValue::Bool(index_set_values(&recv)?.contains(&value)),
        receiver: recv,
    })
}

fn index_set_insert(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if args.len() != 1 {
        return Err(type_error("IndexSet.insert expects one Int"));
    }
    let value = int_arg(&args[0], "IndexSet.insert")?;
    let mut values = index_set_values(&recv)?;
    let inserted = values.insert(value);
    Ok(Outcome {
        result: SwiftValue::tuple_labeled(
            vec![SwiftValue::Bool(inserted), SwiftValue::int(value)],
            vec![Some("inserted".into()), Some("memberAfterInsert".into())],
        ),
        receiver: index_set_value(values),
    })
}

/// `remove(_:)` — drop `value`, returning the removed element or `nil`.
fn index_set_remove(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if args.len() != 1 {
        return Err(type_error("IndexSet.remove expects one Int"));
    }
    let value = int_arg(&args[0], "IndexSet.remove")?;
    let mut values = index_set_values(&recv)?;
    let removed = values.remove(&value);
    Ok(Outcome {
        result: if removed {
            SwiftValue::int(value)
        } else {
            SwiftValue::Nil
        },
        receiver: index_set_value(values),
    })
}

/// `removeAll()` — empty the set.
fn index_set_remove_all(
    _ctx: &mut dyn StdContext,
    _recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !args.is_empty() {
        return Err(type_error("IndexSet.removeAll expects no arguments"));
    }
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: index_set_value(BTreeSet::new()),
    })
}

/// `update(with:)` — insert `value`, returning the equal member it replaced
/// (`value` itself if already present, else `nil`).
fn index_set_update(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if args.len() != 1 {
        return Err(type_error("IndexSet.update expects one Int"));
    }
    let value = int_arg(&args[0], "IndexSet.update")?;
    let mut values = index_set_values(&recv)?;
    let existed = !values.insert(value);
    Ok(Outcome {
        result: if existed {
            SwiftValue::int(value)
        } else {
            SwiftValue::Nil
        },
        receiver: index_set_value(values),
    })
}

/// The other operand of a set-algebra method, as a sorted integer set.
fn other_index_set(args: &[SwiftValue], context: &str) -> Result<BTreeSet<i128>, StdError> {
    if args.len() != 1 {
        return Err(type_error(format!("{context} expects one IndexSet")));
    }
    index_set_values(&args[0])
}

fn index_set_union(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let other = other_index_set(&args, "IndexSet.union")?;
    let result = index_set_values(&recv)?.union(&other).copied().collect();
    Ok(Outcome {
        result: index_set_value(result),
        receiver: recv,
    })
}

fn index_set_intersection(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let other = other_index_set(&args, "IndexSet.intersection")?;
    let result = index_set_values(&recv)?
        .intersection(&other)
        .copied()
        .collect();
    Ok(Outcome {
        result: index_set_value(result),
        receiver: recv,
    })
}

fn index_set_symmetric_difference(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let other = other_index_set(&args, "IndexSet.symmetricDifference")?;
    let result = index_set_values(&recv)?
        .symmetric_difference(&other)
        .copied()
        .collect();
    Ok(Outcome {
        result: index_set_value(result),
        receiver: recv,
    })
}

fn index_set_form_union(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let other = other_index_set(&args, "IndexSet.formUnion")?;
    let result = index_set_values(&recv)?.union(&other).copied().collect();
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: index_set_value(result),
    })
}

fn index_set_form_intersection(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let other = other_index_set(&args, "IndexSet.formIntersection")?;
    let result = index_set_values(&recv)?
        .intersection(&other)
        .copied()
        .collect();
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: index_set_value(result),
    })
}

fn index_set_form_symmetric_difference(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let other = other_index_set(&args, "IndexSet.formSymmetricDifference")?;
    let result = index_set_values(&recv)?
        .symmetric_difference(&other)
        .copied()
        .collect();
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: index_set_value(result),
    })
}

/// Shared helper for the `integer{Greater,Less}Than[OrEqualTo]` family:
/// return the closest member matching `pred` relative to `value`, or `nil`.
fn index_set_nearest(
    recv: &SwiftValue,
    args: &[SwiftValue],
    context: &str,
    pick_max: bool,
    pred: impl Fn(i128, i128) -> bool,
) -> Result<SwiftValue, StdError> {
    if args.len() != 1 {
        return Err(type_error(format!("{context} expects one Int")));
    }
    let value = int_arg(&args[0], context)?;
    let values = index_set_values(recv)?;
    let candidate = values.iter().copied().filter(|&m| pred(m, value));
    let found = if pick_max {
        candidate.max()
    } else {
        candidate.min()
    };
    Ok(found.map(SwiftValue::int).unwrap_or(SwiftValue::Nil))
}

fn index_set_integer_greater_than(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let result = index_set_nearest(
        &recv,
        &args,
        "IndexSet.integerGreaterThan",
        false,
        |m, v| m > v,
    )?;
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

fn index_set_integer_less_than(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let result = index_set_nearest(&recv, &args, "IndexSet.integerLessThan", true, |m, v| m < v)?;
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

fn index_set_integer_ge(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let result = index_set_nearest(
        &recv,
        &args,
        "IndexSet.integerGreaterThanOrEqualTo",
        false,
        |m, v| m >= v,
    )?;
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

fn index_set_integer_le(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let result = index_set_nearest(
        &recv,
        &args,
        "IndexSet.integerLessThanOrEqualTo",
        true,
        |m, v| m <= v,
    )?;
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockContext {
        out: Vec<u8>,
        now: f64,
    }

    impl MockContext {
        fn new(now: f64) -> Self {
            Self {
                out: Vec::new(),
                now,
            }
        }
    }

    impl StdContext for MockContext {
        fn call_closure(&mut self, _id: usize, _args: Vec<SwiftValue>) -> StdResult {
            Err(type_error("closures are unsupported in MockContext"))
        }

        fn out(&mut self) -> &mut dyn std::io::Write {
            &mut self.out
        }

        fn now_unix_seconds(&mut self) -> f64 {
            self.now
        }
    }

    fn date_ref_seconds(value: &SwiftValue) -> f64 {
        date_seconds(value).expect("Date value")
    }

    #[test]
    fn date_initializers_store_reference_seconds() {
        let mut ctx = MockContext::new(REFERENCE_DATE_UNIX_OFFSET + 10.0);

        let now = date_init(&mut ctx, Vec::new()).expect("Date()");
        assert_eq!(date_ref_seconds(&now), 10.0);

        let unix = date_init(
            &mut ctx,
            vec![Arg {
                label: Some("timeIntervalSince1970".into()),
                value: SwiftValue::Double(REFERENCE_DATE_UNIX_OFFSET + 25.0),
            }],
        )
        .expect("Date(timeIntervalSince1970:)");
        assert_eq!(date_ref_seconds(&unix), 25.0);

        let since = date_init(
            &mut ctx,
            vec![
                Arg {
                    label: Some("timeInterval".into()),
                    value: SwiftValue::Double(5.0),
                },
                Arg {
                    label: Some("since".into()),
                    value: unix.clone(),
                },
            ],
        )
        .expect("Date(timeInterval:since:)");
        assert_eq!(date_ref_seconds(&since), 30.0);
    }

    #[test]
    fn date_methods_support_arithmetic_and_compare() {
        let mut ctx = MockContext::new(REFERENCE_DATE_UNIX_OFFSET);
        let base = date_value(100.0);
        let later = date_value(125.0);

        let diff = date_time_interval_since(&mut ctx, later.clone(), vec![base.clone()])
            .expect("timeIntervalSince")
            .result;
        assert_eq!(diff, SwiftValue::Double(25.0));

        let advanced = date_advanced(&mut ctx, base.clone(), vec![SwiftValue::Double(5.0)])
            .expect("advanced")
            .result;
        assert_eq!(date_ref_seconds(&advanced), 105.0);

        let mutated = date_add_time_interval(&mut ctx, base.clone(), vec![SwiftValue::Double(7.0)])
            .expect("addTimeInterval")
            .receiver;
        assert_eq!(date_ref_seconds(&mutated), 107.0);

        let compared = date_compare(&mut ctx, base.clone(), vec![later])
            .expect("compare")
            .result;
        match compared {
            SwiftValue::Enum(result) => assert_eq!(result.case, "orderedAscending"),
            other => panic!("expected ComparisonResult, got {}", other.type_name()),
        }
    }

    #[test]
    fn date_description_renders_utc_instant() {
        // 2001-01-01 00:00:10 UTC == reference offset 10.0.
        let date = date_value(10.0);
        assert_eq!(
            date_description(date.clone()).unwrap(),
            SwiftValue::Str("2001-01-01 00:00:10 +0000".into())
        );
        // 2024-06-29 09:41:00 UTC.
        let later = date_value(crate::calendar::ref_seconds_from_ymdhms(
            2024, 6, 29, 9, 41, 0,
        ));
        assert_eq!(
            date_description(later).unwrap(),
            SwiftValue::Str("2024-06-29 09:41:00 +0000".into())
        );
    }

    #[test]
    fn date_hash_value_is_consistent_with_equality() {
        assert_eq!(
            date_hash_value(date_value(10.0)).unwrap(),
            date_hash_value(date_value(10.0)).unwrap()
        );
        assert_ne!(
            date_hash_value(date_value(10.0)).unwrap(),
            date_hash_value(date_value(11.0)).unwrap()
        );
        // `0.0 == -0.0` under `Date ==`, so they must hash equally.
        assert_eq!(
            date_hash_value(date_value(0.0)).unwrap(),
            date_hash_value(date_value(-0.0)).unwrap()
        );
    }

    #[test]
    fn date_description_truncates_and_handles_pre_reference() {
        // Fractional seconds truncate to whole seconds.
        assert_eq!(
            date_description(date_value(10.9)).unwrap(),
            SwiftValue::Str("2001-01-01 00:00:10 +0000".into())
        );
        // Just before the reference epoch wraps the civil date back a day.
        assert_eq!(
            date_description(date_value(-0.1)).unwrap(),
            SwiftValue::Str("2000-12-31 23:59:59 +0000".into())
        );
    }

    fn labeled(label: &str, value: i128) -> Arg {
        Arg {
            label: Some(label.into()),
            value: SwiftValue::int(value),
        }
    }

    #[test]
    fn date_components_partial_construction_reads_back() {
        let mut ctx = MockContext::new(0.0);
        let dc = date_components_init(
            &mut ctx,
            vec![
                labeled("year", 2024),
                labeled("month", 6),
                labeled("day", 29),
            ],
        )
        .expect("DateComponents");

        assert_eq!(
            date_components_get(&dc, "year").unwrap(),
            SwiftValue::int(2024)
        );
        assert_eq!(
            date_components_get(&dc, "month").unwrap(),
            SwiftValue::int(6)
        );
        assert_eq!(
            date_components_get(&dc, "day").unwrap(),
            SwiftValue::int(29)
        );
        // Unset components read back as nil.
        assert_eq!(date_components_get(&dc, "hour").unwrap(), SwiftValue::Nil);
    }

    #[test]
    fn date_components_is_valid_date_checks_ymd_ranges() {
        let mut ctx = MockContext::new(0.0);
        let valid = date_components_init(
            &mut ctx,
            vec![
                labeled("year", 2024),
                labeled("month", 6),
                labeled("day", 29),
            ],
        )
        .unwrap();
        assert_eq!(
            date_components_is_valid_date(valid).unwrap(),
            SwiftValue::Bool(true)
        );

        let bad_month = date_components_init(
            &mut ctx,
            vec![
                labeled("year", 2024),
                labeled("month", 13),
                labeled("day", 5),
            ],
        )
        .unwrap();
        assert_eq!(
            date_components_is_valid_date(bad_month).unwrap(),
            SwiftValue::Bool(false)
        );

        let missing_day = date_components_init(&mut ctx, vec![labeled("year", 2024)]).unwrap();
        assert_eq!(
            date_components_is_valid_date(missing_day).unwrap(),
            SwiftValue::Bool(false)
        );
    }

    #[test]
    fn base64_round_trips() {
        for case in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
            let encoded = base64_encode(case.as_bytes());
            let decoded = base64_decode(&encoded).expect("decodes");
            assert_eq!(decoded, case.as_bytes(), "round-trip for {case:?}");
        }
        assert_eq!(base64_encode(b"Hi"), "SGk=");
        assert_eq!(base64_decode("SGk=").unwrap(), b"Hi");
        // Malformed inputs reject.
        assert!(base64_decode("SGk").is_none()); // wrong length
        assert!(base64_decode("@@@@").is_none()); // bad alphabet
        assert!(base64_decode("====").is_none()); // all padding
        assert!(base64_decode("AA==AAAA").is_none()); // padding before final chunk
        assert!(base64_decode("A===").is_none()); // 3 padding chars
                                                  // Empty decodes to empty Data, matching Foundation.
        assert_eq!(base64_decode("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn is_valid_date_respects_month_lengths() {
        let mut ctx = MockContext::new(0.0);
        let leap = date_components_init(
            &mut ctx,
            vec![
                labeled("year", 2024),
                labeled("month", 2),
                labeled("day", 29),
            ],
        )
        .unwrap();
        assert_eq!(
            date_components_is_valid_date(leap).unwrap(),
            SwiftValue::Bool(true)
        );
        let non_leap = date_components_init(
            &mut ctx,
            vec![
                labeled("year", 2023),
                labeled("month", 2),
                labeled("day", 29),
            ],
        )
        .unwrap();
        assert_eq!(
            date_components_is_valid_date(non_leap).unwrap(),
            SwiftValue::Bool(false)
        );
    }

    #[test]
    fn date_components_set_value_updates_component() {
        let mut ctx = MockContext::new(0.0);
        let dc = date_components_init(&mut ctx, vec![labeled("year", 2024)]).unwrap();
        let component = SwiftValue::Enum(Rc::new(EnumObj {
            type_name: "Calendar.Component".into(),
            case: "month".into(),
            payload: Vec::new(),
        }));
        let updated =
            date_components_set_value(&mut ctx, dc, vec![SwiftValue::int(7), component.clone()])
                .unwrap()
                .receiver;
        assert_eq!(
            date_components_get(&updated, "month").unwrap(),
            SwiftValue::int(7)
        );

        let read = date_components_value(&mut ctx, updated, vec![component])
            .unwrap()
            .result;
        assert_eq!(read, SwiftValue::int(7));
    }

    #[test]
    fn date_components_stores_era_and_day_of_year() {
        let mut ctx = MockContext::new(0.0);
        let dc =
            date_components_init(&mut ctx, vec![labeled("era", 1), labeled("year", 2024)]).unwrap();
        // `era` is now a readable component rather than an ignored label.
        assert_eq!(date_components_get(&dc, "era").unwrap(), SwiftValue::int(1));
        // `dayOfYear` defaults to nil when unset.
        assert_eq!(
            date_components_get(&dc, "dayOfYear").unwrap(),
            SwiftValue::Nil
        );
    }
}

#[cfg(test)]
mod coverage_dump {
    #[test]
    fn dump_registered_keys() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = root.join("frameworks/foundation/registered_keys.txt");
        let body = super::registered_keys().join("\n") + "\n";
        std::fs::write(&path, body).expect("write registered_keys.txt");
    }
}
