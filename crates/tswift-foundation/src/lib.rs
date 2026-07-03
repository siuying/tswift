//! tswift-foundation — native Foundation value builtins.
//!
//! The crate mirrors the `tswift-std` registry seam: install once into an
//! interpreter, expose live `registered_keys()` for coverage tooling, and keep
//! behaviour slices small enough to validate with CLI golden fixtures.

mod calendar;
mod datestyle;
mod decimal;
mod formatter;
mod json;
mod measurement;
mod network;
mod numberformatter;
mod plist;
mod url;
mod urlsession;

use std::{collections::BTreeSet, rc::Rc};

use tswift_core::{
    Arg, BuiltinReceiver, EnumObj, EvalError, Interpreter, IntrinsicFn, LabeledMethodEntry,
    MethodEntry, Outcome, StdContext, StdError, StdResult, StructObj, SwiftValue,
};

const REFERENCE_DATE_UNIX_OFFSET: f64 = 978_307_200.0;
const DISTANT_PAST_REFERENCE_SECONDS: f64 = -63_113_904_000.0;
const DISTANT_FUTURE_REFERENCE_SECONDS: f64 = 63_113_904_000.0;

/// Register every currently-supported Foundation builtin into `interp`.
pub fn install(interp: &mut Interpreter<'_>) {
    url::install(interp);
    network::install(interp);
    urlsession::install(interp);
    calendar::install(interp);
    datestyle::install(interp);
    formatter::install(interp);
    decimal::install(interp);
    json::install(interp);
    numberformatter::install(interp);
    measurement::install(interp);
    plist::install(interp);
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
    interp.register_property(
        BuiltinReceiver::DateComponents,
        "calendar",
        date_components_calendar,
    );
    interp.register_property(
        BuiltinReceiver::DateComponents,
        "timeZone",
        date_components_time_zone,
    );
    interp.register_property(
        BuiltinReceiver::DateComponents,
        "isLeapMonth",
        date_components_is_leap_month,
    );
    interp.register_property(
        BuiltinReceiver::DateComponents,
        "description",
        date_components_description,
    );
    interp.register_property(
        BuiltinReceiver::DateComponents,
        "debugDescription",
        date_components_description,
    );
    interp.register_property(
        BuiltinReceiver::DateComponents,
        "hashValue",
        date_components_hash_value,
    );
    interp.register_property(
        BuiltinReceiver::DateComponents,
        "date",
        date_components_date,
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
    interp.register_property(BuiltinReceiver::Data, "debugDescription", data_description);
    interp.register_property(BuiltinReceiver::Data, "hashValue", data_hash_value);
    interp.register_property(BuiltinReceiver::Data, "startIndex", data_start_index);
    interp.register_property(BuiltinReceiver::Data, "endIndex", data_end_index);
    interp.register_property(BuiltinReceiver::Data, "indices", data_indices);
    for (name, mutating, func) in [
        ("append", true, data_append as IntrinsicFn),
        ("base64EncodedString", false, data_base64_encoded_string),
        ("base64EncodedData", false, data_base64_encoded_data),
        ("subdata", false, data_subdata),
        ("removeAll", true, data_remove_all),
        ("replaceSubrange", true, data_replace_subrange),
        ("reserveCapacity", true, data_reserve_capacity),
        ("resetBytes", true, data_reset_bytes),
        ("range", false, data_range_of),
        ("makeIterator", false, data_make_iterator),
    ] {
        interp.register_intrinsic(BuiltinReceiver::Data, name, MethodEntry { mutating, func });
    }
    // `index(after:)` and `index(before:)` are label-sensitive overloads.
    interp.register_labeled_intrinsic(
        BuiltinReceiver::Data,
        "index",
        LabeledMethodEntry {
            mutating: false,
            func: data_index_labeled,
        },
    );

    interp.register_free_fn("UUID", uuid_init);
    interp.register_property(BuiltinReceiver::UUID, "uuidString", uuid_string);
    interp.register_property(BuiltinReceiver::UUID, "description", uuid_description);
    interp.register_property(BuiltinReceiver::UUID, "debugDescription", uuid_description);
    interp.register_property(BuiltinReceiver::UUID, "hashValue", uuid_hash_value);

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
    interp.register_property(
        BuiltinReceiver::IndexPath,
        "hashValue",
        index_path_hash_value,
    );
    interp.register_property(
        BuiltinReceiver::IndexPath,
        "description",
        index_path_description,
    );
    interp.register_property(
        BuiltinReceiver::IndexPath,
        "debugDescription",
        index_path_description,
    );
    for (name, mutating, func) in [
        ("dropLast", false, index_path_drop_last as IntrinsicFn),
        ("makeIterator", false, index_path_make_iterator),
        ("compare", false, index_path_compare),
        ("==", false, index_path_equal),
    ] {
        interp.register_intrinsic(
            BuiltinReceiver::IndexPath,
            name,
            MethodEntry { mutating, func },
        );
    }
    interp.register_labeled_intrinsic(
        BuiltinReceiver::IndexPath,
        "index",
        LabeledMethodEntry {
            mutating: false,
            func: index_path_index_labeled,
        },
    );

    // Register UUID `<` so it appears in coverage keys; actual comparison
    // is handled by ops::binary via uuid_binary.
    interp.register_intrinsic(
        BuiltinReceiver::UUID,
        "<",
        MethodEntry {
            mutating: false,
            func: uuid_less_than,
        },
    );

    interp.register_free_fn("IndexSet", index_set_init);
    interp.register_property(BuiltinReceiver::IndexSet, "count", index_set_count);
    interp.register_property(BuiltinReceiver::IndexSet, "isEmpty", index_set_is_empty);
    interp.register_property(BuiltinReceiver::IndexSet, "first", index_set_first);
    interp.register_property(BuiltinReceiver::IndexSet, "last", index_set_last);
    interp.register_property(BuiltinReceiver::IndexSet, "hashValue", index_set_hash_value);
    interp.register_property(
        BuiltinReceiver::IndexSet,
        "description",
        index_set_description,
    );
    interp.register_property(
        BuiltinReceiver::IndexSet,
        "debugDescription",
        index_set_description,
    );
    interp.register_property(BuiltinReceiver::IndexSet, "rangeView", index_set_range_view);
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
    let non_mutating: [(&str, IntrinsicFn); 11] = [
        ("union", index_set_union),
        ("intersection", index_set_intersection),
        ("symmetricDifference", index_set_symmetric_difference),
        ("integerGreaterThan", index_set_integer_greater_than),
        ("integerLessThan", index_set_integer_less_than),
        ("integerGreaterThanOrEqualTo", index_set_integer_ge),
        ("integerLessThanOrEqualTo", index_set_integer_le),
        ("==", index_set_equal),
        ("intersects", index_set_intersects),
        ("makeIterator", index_set_make_iterator),
        ("filteredIndexSet", index_set_filtered),
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
    let mutating: [(&str, IntrinsicFn); 4] = [
        ("formUnion", index_set_form_union),
        ("formIntersection", index_set_form_intersection),
        (
            "formSymmetricDifference",
            index_set_form_symmetric_difference,
        ),
        ("shift", index_set_shift),
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
    // JSONEncoder / JSONDecoder are handled by the interpreter's built-in
    // coding machinery (not the standard registry), so their keys are injected
    // manually here so the coverage tooling counts them.
    keys.extend(json::registered_keys());
    // PropertyListEncoder is handled by the interpreter's built-in coding
    // machinery (same pattern as JSONEncoder); inject keys manually.
    keys.extend(plist::registered_keys());
    // Date.FormatStyle enum cases + static values are not auto-detected by the
    // registry filter above; inject them manually.
    keys.extend(datestyle::extra_registered_keys());
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

/// Extra (non-integer) fields stored on a `DateComponents` value.
/// `calendar` (Calendar?), `timeZone` (TimeZone?), `isLeapMonth` (Bool?).
const DATE_COMPONENT_EXTRA_FIELDS: &[&str] = &["calendar", "timeZone", "isLeapMonth"];

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
    // Int-valued calendar components.
    let mut fields: Vec<(String, SwiftValue)> = DATE_COMPONENT_FIELDS
        .iter()
        .map(|name| ((*name).to_string(), SwiftValue::Nil))
        .collect();
    // Extra non-integer fields initialised to nil.
    for name in DATE_COMPONENT_EXTRA_FIELDS {
        fields.push(((*name).to_string(), SwiftValue::Nil));
    }
    for arg in &args {
        let Some(label) = arg.label.as_deref() else {
            return Err(type_error(
                "DateComponents initializer requires labeled arguments",
            ));
        };
        if label == "calendar" {
            let value = match &arg.value {
                SwiftValue::Nil => SwiftValue::Nil,
                SwiftValue::Struct(obj) if obj.type_name == "Calendar" => arg.value.clone(),
                other => {
                    return Err(type_error(format!(
                        "DateComponents(calendar:) expects Calendar, got {}",
                        other.type_name()
                    )))
                }
            };
            if let Some(slot) = fields.iter_mut().find(|(n, _)| n == "calendar") {
                slot.1 = value;
            }
            continue;
        }
        if label == "timeZone" {
            let value = match &arg.value {
                SwiftValue::Nil => SwiftValue::Nil,
                SwiftValue::Struct(obj) if obj.type_name == "TimeZone" => arg.value.clone(),
                other => {
                    return Err(type_error(format!(
                        "DateComponents(timeZone:) expects TimeZone, got {}",
                        other.type_name()
                    )))
                }
            };
            if let Some(slot) = fields.iter_mut().find(|(n, _)| n == "timeZone") {
                slot.1 = value;
            }
            continue;
        }
        if label == "isLeapMonth" {
            let value = match &arg.value {
                SwiftValue::Nil => SwiftValue::Nil,
                SwiftValue::Bool(b) => SwiftValue::Bool(*b),
                other => {
                    return Err(type_error(format!(
                        "DateComponents(isLeapMonth:) expects Bool?, got {}",
                        other.type_name()
                    )))
                }
            };
            if let Some(slot) = fields.iter_mut().find(|(n, _)| n == "isLeapMonth") {
                slot.1 = value;
            }
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

// ---------------------------------------------------------------------------
// DateComponents extended properties
// ---------------------------------------------------------------------------

/// `DateComponents.calendar` — the associated `Calendar?` (nil if not stored).
fn date_components_calendar(recv: SwiftValue) -> StdResult {
    let obj = date_components_obj(&recv)?;
    Ok(obj.get("calendar").cloned().unwrap_or(SwiftValue::Nil))
}

/// `DateComponents.timeZone` — the associated `TimeZone?` (nil if not stored).
fn date_components_time_zone(recv: SwiftValue) -> StdResult {
    let obj = date_components_obj(&recv)?;
    Ok(obj.get("timeZone").cloned().unwrap_or(SwiftValue::Nil))
}

/// `DateComponents.isLeapMonth` — `Bool?` (nil when not specified).
fn date_components_is_leap_month(recv: SwiftValue) -> StdResult {
    let obj = date_components_obj(&recv)?;
    Ok(obj.get("isLeapMonth").cloned().unwrap_or(SwiftValue::Nil))
}

/// Description field order that matches Foundation's output:
/// era, year, month, day, hour, minute, second, nanosecond,
/// weekday, weekdayOrdinal, quarter, weekOfMonth, weekOfYear,
/// yearForWeekOfYear — then isLeapMonth at the end.
const DATE_COMPONENT_DESC_ORDER: &[&str] = &[
    "era",
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
];

/// Extract the `description` string from a stored `TimeZone` struct.
fn tz_description_str(tz_val: &SwiftValue) -> Option<String> {
    if let SwiftValue::Struct(tz) = tz_val {
        if tz.type_name == "TimeZone" {
            if let Some(SwiftValue::Str(s)) = tz.get("description") {
                return Some(s.clone());
            }
        }
    }
    None
}

/// `DateComponents.description` / `debugDescription`.
///
/// Matches Foundation's format byte-for-byte:
/// - Calendar block (`"calendar: ... "`) if calendar is stored.
/// - TimeZone entry (`"timeZone: <desc> "`) if timeZone is stored,
///   regardless of whether calendar is also stored (Foundation always emits
///   both when both are present).
/// - Integer component fields in canonical order.
/// - `isLeapMonth` at the end.
fn date_components_description(recv: SwiftValue) -> StdResult {
    let obj = date_components_obj(&recv)?;
    let mut parts = String::new();

    // Calendar block: "calendar: <cal.description> "
    if let Some(SwiftValue::Struct(cal)) = obj.get("calendar") {
        if cal.type_name == "Calendar" {
            let cal_desc = crate::calendar::calendar_description_str(cal);
            parts.push_str(&format!("calendar: {cal_desc} "));
        }
    }

    // TimeZone entry: "timeZone: <tz.description> "
    // Appears when timeZone is set, even if calendar is also set (Case C).
    if let Some(tz_val) = obj.get("timeZone") {
        if let Some(tz_desc) = tz_description_str(tz_val) {
            parts.push_str(&format!("timeZone: {tz_desc} "));
        }
    }

    // Integer component fields.
    for field in DATE_COMPONENT_DESC_ORDER {
        if let Some(SwiftValue::Int(i)) = obj.get(*field) {
            parts.push_str(&format!("{field}: {} ", i.raw));
        }
    }

    // Bool isLeapMonth at the end.
    if let Some(SwiftValue::Bool(b)) = obj.get("isLeapMonth") {
        parts.push_str(&format!("isLeapMonth: {b} "));
    }

    Ok(SwiftValue::Str(parts))
}

/// `DateComponents.hashValue` — hash of all non-nil component fields.
///
/// Each int field contributes a present-marker byte + 8 bytes of its value;
/// absent fields contribute a zero byte.  `isLeapMonth` adds a separate
/// two-byte sequence.  This is consistent with `==` (which compares all
/// fields).
fn date_components_hash_value(recv: SwiftValue) -> StdResult {
    let obj = date_components_obj(&recv)?;
    let mut bytes: Vec<u8> = Vec::new();
    for field in DATE_COMPONENT_FIELDS {
        match obj.get(*field) {
            Some(SwiftValue::Int(i)) => {
                bytes.push(1);
                bytes.extend_from_slice(&(i.raw as i64).to_le_bytes());
            }
            _ => bytes.push(0),
        }
    }
    // isLeapMonth
    match obj.get("isLeapMonth") {
        Some(SwiftValue::Bool(b)) => {
            bytes.push(2);
            bytes.push(if *b { 1 } else { 0 });
        }
        _ => bytes.push(0),
    }
    Ok(SwiftValue::int(fnv1a_hash(&bytes)))
}

/// `DateComponents.date` — compute a `Date?` from the stored calendar and
/// component values.
///
/// Returns `nil` when no calendar is stored.  When a calendar is present,
/// missing y/m/d default to 1 and missing time fields default to 0 —
/// matching the behaviour of `Calendar.date(from:)`.
fn date_components_date(recv: SwiftValue) -> StdResult {
    let obj = date_components_obj(&recv)?;
    // Require a stored Calendar.
    match obj.get("calendar") {
        Some(SwiftValue::Struct(cal)) if cal.type_name == "Calendar" => {}
        _ => return Ok(SwiftValue::Nil),
    }
    let get_int = |field: &str| -> Option<i64> {
        match obj.get(field) {
            Some(SwiftValue::Int(i)) => Some(i.raw as i64),
            _ => None,
        }
    };
    let year = get_int("year").unwrap_or(1);
    let month = get_int("month").unwrap_or(1);
    let day = get_int("day").unwrap_or(1);
    let hour = get_int("hour").unwrap_or(0);
    let minute = get_int("minute").unwrap_or(0);
    let second = get_int("second").unwrap_or(0);
    let seconds = crate::calendar::ref_seconds_from_ymdhms(year, month, day, hour, minute, second);
    Ok(date_value(seconds))
}

pub(crate) fn data_value(bytes: Vec<u8>) -> SwiftValue {
    let elements = bytes
        .into_iter()
        .map(|b| SwiftValue::int(i128::from(b)))
        .collect();
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "Data".into(),
        fields: vec![("_bytes".into(), SwiftValue::Array(Rc::new(elements)))],
    }))
}

pub(crate) fn data_bytes(value: &SwiftValue) -> Result<Vec<u8>, StdError> {
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
        return Ok(match tswift_core::base64::decode(s) {
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
    let encoded = tswift_core::base64::encode(&data_bytes(&recv)?);
    Ok(Outcome {
        result: SwiftValue::Str(encoded),
        receiver: recv,
    })
}

fn data_base64_encoded_data(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !args.is_empty() {
        return Err(type_error("base64EncodedData() takes no arguments"));
    }
    // The base64 text encoded as its ASCII bytes, wrapped back into `Data`.
    let encoded = tswift_core::base64::encode(&data_bytes(&recv)?);
    Ok(Outcome {
        result: data_value(encoded.into_bytes()),
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

/// `Data.replaceSubrange(_:with:)` — splice replacement bytes (from `Data` or
/// `[UInt8]`) into the receiver, shifting bytes as needed.
fn data_replace_subrange(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let mut bytes = data_bytes(&recv)?;
    let [range_val, replacement_val] = args.as_slice() else {
        return Err(type_error(
            "replaceSubrange(_:with:) expects a Range<Int> and replacement bytes",
        ));
    };
    let (start, end) = match range_val {
        SwiftValue::Range { lo, hi, inclusive } => {
            let end = if *inclusive { hi + 1 } else { *hi };
            if *lo < 0 || end < *lo || end as usize > bytes.len() {
                return Err(type_error("replaceSubrange(_:with:) range out of bounds"));
            }
            (*lo as usize, end as usize)
        }
        _ => return Err(type_error("replaceSubrange(_:with:) expects a Range<Int>")),
    };
    let replacement = match replacement_val {
        SwiftValue::Struct(obj) if obj.type_name == "Data" => data_bytes(replacement_val)?,
        SwiftValue::Array(items) => items
            .iter()
            .map(byte_from_value)
            .collect::<Result<Vec<_>, _>>()?,
        _ => {
            return Err(type_error(
                "replaceSubrange(_:with:) replacement must be Data or [UInt8]",
            ))
        }
    };
    bytes.splice(start..end, replacement);
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: data_value(bytes),
    })
}

/// `Data.reserveCapacity(_:)` — no-op; the interpreter's `Vec<u8>` already
/// grows on demand and there is no benefit in pre-allocating from Swift.
fn data_reserve_capacity(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    // Require exactly one non-negative Int argument.
    let [cap] = args.as_slice() else {
        return Err(type_error(
            "reserveCapacity(_:) requires exactly one Int argument",
        ));
    };
    match cap {
        SwiftValue::Int(i) if i.raw >= 0 => {}
        SwiftValue::Int(i) => {
            return Err(type_error(format!(
                "reserveCapacity(_:) requires a non-negative Int, got {}",
                i.raw
            )))
        }
        other => {
            return Err(type_error(format!(
                "reserveCapacity(_:) expects Int, got {}",
                other.type_name()
            )))
        }
    }
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: recv,
    })
}

/// `Data.resetBytes(in:)` — zero all bytes in `range`.
fn data_reset_bytes(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let mut bytes = data_bytes(&recv)?;
    let [range_val] = args.as_slice() else {
        return Err(type_error(
            "resetBytes(in:) expects one Range<Int> argument",
        ));
    };
    let (start, end) = match range_val {
        SwiftValue::Range { lo, hi, inclusive } => {
            let end = if *inclusive { hi + 1 } else { *hi };
            if *lo < 0 || end < *lo || end as usize > bytes.len() {
                return Err(type_error("resetBytes(in:) range out of bounds"));
            }
            (*lo as usize, end as usize)
        }
        _ => return Err(type_error("resetBytes(in:) expects a Range<Int>")),
    };
    for b in &mut bytes[start..end] {
        *b = 0;
    }
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: data_value(bytes),
    })
}

/// `Data.range(of:)` — find the first occurrence of `needle` in the receiver.
/// Returns `Range<Int>?`; `nil` when absent or when `needle` is empty (matching
/// Swift's Foundation behaviour).
fn data_range_of(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let haystack = data_bytes(&recv)?;
    let [needle_val] = args.as_slice() else {
        return Err(type_error("range(of:) expects one Data argument"));
    };
    let needle = data_bytes(needle_val)?;
    // Swift returns nil for an empty needle.
    let result = if needle.is_empty() {
        SwiftValue::Nil
    } else {
        haystack
            .windows(needle.len())
            .position(|w| w == needle.as_slice())
            .map(|pos| SwiftValue::Range {
                lo: pos as i128,
                hi: (pos + needle.len()) as i128,
                inclusive: false,
            })
            .unwrap_or(SwiftValue::Nil)
    };
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

/// `Data.index(after:)` and `Data.index(before:)` — label-aware dispatch.
/// `Data.Index` is `Int`, so these are trivially `i + 1` / `i - 1`.
fn data_index_labeled(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let [arg] = args.as_slice() else {
        return Ok(None); // wrong arity — let other overloads try
    };
    let i = match &arg.value {
        SwiftValue::Int(i) => i.raw,
        _ => return Err(type_error("index(after:)/index(before:) expects Int")),
    };
    let result = match arg.label.as_deref() {
        Some("after") => SwiftValue::int(i + 1),
        Some("before") => SwiftValue::int(i - 1),
        _ => return Ok(None), // unrecognised label — fall through
    };
    Ok(Some(Outcome {
        result,
        receiver: recv,
    }))
}

/// `Data.makeIterator()` — returns the data itself (Data is its own iterator
/// over bytes; the for-in machinery uses `materialize_builtin_sequence`).
fn data_make_iterator(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !args.is_empty() {
        return Err(type_error("makeIterator() takes no arguments"));
    }
    let result = recv.clone();
    Ok(Outcome {
        result,
        receiver: recv,
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

/// FNV-1a 64-bit hash, narrowed to the platform `Int`. Used to give builtin
/// value types a `hashValue` consistent with their `==`.
pub(crate) fn fnv1a_hash(bytes: &[u8]) -> i128 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    (hash as i64) as i128
}

fn uuid_hash_value(recv: SwiftValue) -> StdResult {
    // Equal UUIDs share the canonical string, so hashing it matches `==`.
    let SwiftValue::Str(s) = uuid_string(recv)? else {
        return Err(type_error("malformed UUID value"));
    };
    Ok(SwiftValue::int(fnv1a_hash(s.as_bytes())))
}

fn data_hash_value(recv: SwiftValue) -> StdResult {
    // `Data ==` compares the byte sequence, so hash the bytes.
    Ok(SwiftValue::int(fnv1a_hash(&data_bytes(&recv)?)))
}

/// `Data.startIndex` — always `0` (a `Data` is a zero-based byte collection).
fn data_start_index(recv: SwiftValue) -> StdResult {
    data_bytes(&recv)?;
    Ok(SwiftValue::int(0))
}

/// `Data.endIndex` — the past-the-end position, i.e. the byte count.
fn data_end_index(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(data_bytes(&recv)?.len() as i128))
}

/// `Data.indices` — the half-open range `0..<count`.
fn data_indices(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Range {
        lo: 0,
        hi: data_bytes(&recv)?.len() as i128,
        inclusive: false,
    })
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

/// Hash a sequence of integers (used by `IndexPath`/`IndexSet`), consistent
/// with their element-wise `==`.
fn hash_int_sequence(values: impl IntoIterator<Item = i128>) -> i128 {
    let mut bytes = Vec::new();
    for value in values {
        bytes.extend_from_slice(&(value as i64).to_le_bytes());
    }
    fnv1a_hash(&bytes)
}

fn index_path_hash_value(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(hash_int_sequence(index_path_indexes(
        &recv,
    )?)))
}

fn index_set_hash_value(recv: SwiftValue) -> StdResult {
    // BTreeSet iterates in sorted order, matching set equality.
    Ok(SwiftValue::int(hash_int_sequence(index_set_values(&recv)?)))
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

/// `IndexPath.description` / `debugDescription` — format `[1, 2, 3]`.
fn index_path_description(recv: SwiftValue) -> StdResult {
    let indexes = index_path_indexes(&recv)?;
    let inner: Vec<String> = indexes.iter().map(|i| i.to_string()).collect();
    Ok(SwiftValue::Str(format!("[{}]", inner.join(", ")).into()))
}

/// `IndexPath.makeIterator()` — returns self; for-in uses
/// `materialize_builtin_sequence` which handles IndexPath structs.
fn index_path_make_iterator(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !args.is_empty() {
        return Err(type_error("IndexPath.makeIterator expects no arguments"));
    }
    let result = recv.clone();
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

/// `IndexPath.compare(_:) -> ComparisonResult` — lexicographic comparison.
fn index_path_compare(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if args.len() != 1 {
        return Err(type_error("IndexPath.compare expects one IndexPath"));
    }
    let lhs = index_path_indexes(&recv)?;
    let rhs = index_path_indexes(&args[0])?;
    let case = match lhs.cmp(&rhs) {
        std::cmp::Ordering::Less => "orderedAscending",
        std::cmp::Ordering::Greater => "orderedDescending",
        std::cmp::Ordering::Equal => "orderedSame",
    };
    Ok(Outcome {
        result: comparison_result(case),
        receiver: recv,
    })
}

/// `IndexPath.==` — registered for coverage key; actual equality uses struct ==.
fn index_path_equal(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if args.len() != 1 {
        return Err(type_error("IndexPath.== expects one IndexPath"));
    }
    let lhs = index_path_indexes(&recv)?;
    let rhs = index_path_indexes(&args[0])?;
    Ok(Outcome {
        result: SwiftValue::Bool(lhs == rhs),
        receiver: recv,
    })
}

/// `IndexPath.index(after:)` / `IndexPath.index(before:)` — labeled method.
fn index_path_index_labeled(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let [arg] = args.as_slice() else {
        return Ok(None); // wrong arity — let other overloads try
    };
    let idx = int_arg(&arg.value, "IndexPath.index")?;
    let result = match arg.label.as_deref() {
        Some("after") => SwiftValue::int(idx + 1),
        Some("before") => {
            if idx == 0 {
                return Err(type_error(
                    "IndexPath.index(before:): index 0 has no predecessor",
                ));
            }
            SwiftValue::int(idx - 1)
        }
        _ => return Ok(None), // unrecognised label — fall through
    };
    Ok(Some(Outcome {
        result,
        receiver: recv,
    }))
}

/// `UUID.<` — registered for coverage; actual comparison goes through ops::binary.
fn uuid_less_than(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if args.len() != 1 {
        return Err(type_error("UUID.< expects one UUID"));
    }
    let lhs = match &recv {
        SwiftValue::Struct(obj) if obj.type_name == "UUID" => match obj.get("uuidString") {
            Some(SwiftValue::Str(s)) => s.clone(),
            _ => return Err(type_error("malformed UUID value")),
        },
        _ => return Err(type_error("UUID.< expects UUID receiver")),
    };
    let rhs = match &args[0] {
        SwiftValue::Struct(obj) if obj.type_name == "UUID" => match obj.get("uuidString") {
            Some(SwiftValue::Str(s)) => s.clone(),
            _ => return Err(type_error("malformed UUID argument")),
        },
        _ => return Err(type_error("UUID.< expects UUID argument")),
    };
    Ok(Outcome {
        result: SwiftValue::Bool(lhs < rhs),
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

/// `description` / `debugDescription` — matches swift-corelibs-foundation:
/// `"\(count) indexes"` unconditionally (even `1 indexes`).
fn index_set_description(recv: SwiftValue) -> StdResult {
    let count = index_set_values(&recv)?.len();
    Ok(SwiftValue::Str(format!("{count} indexes")))
}

/// `rangeView` property — Array of maximal contiguous `Range<Int>` values.
fn index_set_range_view(recv: SwiftValue) -> StdResult {
    let values = index_set_values(&recv)?;
    let mut ranges: Vec<SwiftValue> = Vec::new();
    let mut iter = values.iter().copied();
    if let Some(mut start) = iter.next() {
        let mut end = start + 1;
        for v in iter {
            if v == end {
                end += 1;
            } else {
                ranges.push(SwiftValue::Range {
                    lo: start,
                    hi: end,
                    inclusive: false,
                });
                start = v;
                end = v + 1;
            }
        }
        ranges.push(SwiftValue::Range {
            lo: start,
            hi: end,
            inclusive: false,
        });
    }
    Ok(SwiftValue::Array(Rc::new(ranges)))
}

/// `==` — equality operator (registered so it appears in registered_keys).
/// Struct equality already works through PartialEq; this provides the key.
fn index_set_equal(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if args.len() != 1 {
        return Err(type_error("IndexSet.== expects one IndexSet"));
    }
    let lhs = index_set_values(&recv)?;
    let rhs = index_set_values(&args[0])?;
    Ok(Outcome {
        result: SwiftValue::Bool(lhs == rhs),
        receiver: recv,
    })
}

/// `intersects(integersIn:)` — true if any member falls in `range`.
fn index_set_intersects(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if args.len() != 1 {
        return Err(type_error("IndexSet.intersects expects one Range<Int>"));
    }
    let range_ints = ints_in_range(&args[0], "IndexSet.intersects(integersIn:)")?;
    let values = index_set_values(&recv)?;
    let found = range_ints.iter().any(|i| values.contains(i));
    Ok(Outcome {
        result: SwiftValue::Bool(found),
        receiver: recv,
    })
}

/// `makeIterator()` — returns the IndexSet itself; for-in uses
/// `materialize_builtin_sequence` which handles IndexSet structs.
fn index_set_make_iterator(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !args.is_empty() {
        return Err(type_error("IndexSet.makeIterator expects no arguments"));
    }
    let result = recv.clone();
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

/// `filteredIndexSet(includeInteger:)` — filter members with a closure.
fn index_set_filtered(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if args.len() != 1 {
        return Err(type_error("IndexSet.filteredIndexSet expects one closure"));
    }
    let id = match &args[0] {
        SwiftValue::Closure(id) => *id,
        _ => {
            return Err(type_error(
                "IndexSet.filteredIndexSet expects a closure argument",
            ))
        }
    };
    let values = index_set_values(&recv)?;
    let mut result: BTreeSet<i128> = BTreeSet::new();
    for v in values {
        if ctx
            .call_closure(id, vec![SwiftValue::int(v)])?
            .as_bool()
            .unwrap_or(false)
        {
            result.insert(v);
        }
    }
    Ok(Outcome {
        result: index_set_value(result),
        receiver: recv,
    })
}

/// `shift(startingAt:by:)` — elements >= `start` are shifted by `delta`.
/// Negative delta may produce collisions with existing elements; Foundation
/// removes colliding indices (it does NOT crash for negative delta). We
/// simply rebuild the BTreeSet with all shifted values, letting insertion
/// order resolve duplicates (last write wins via BTreeSet semantics, but
/// since we process in order the shifted value simply lands in the set).
fn index_set_shift(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if args.len() != 2 {
        return Err(type_error(
            "IndexSet.shift expects (startingAt:, by:) arguments",
        ));
    }
    let start = int_arg(&args[0], "IndexSet.shift startingAt")?;
    let delta = int_arg(&args[1], "IndexSet.shift by")?;
    let values = index_set_values(&recv)?;
    let result: BTreeSet<i128> = values
        .into_iter()
        .map(|v| if v >= start { v + delta } else { v })
        .collect();
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: index_set_value(result),
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
    fn data_collection_surface_reports_bounds_and_indices() {
        let d = data_value(vec![10, 20, 30]);
        assert_eq!(data_start_index(d.clone()).unwrap(), SwiftValue::int(0));
        assert_eq!(data_end_index(d.clone()).unwrap(), SwiftValue::int(3));
        assert_eq!(
            data_indices(d).unwrap(),
            SwiftValue::Range {
                lo: 0,
                hi: 3,
                inclusive: false
            }
        );
    }

    #[test]
    fn base64_encoded_data_wraps_ascii_bytes() {
        let mut ctx = MockContext::new(0.0);
        // "ABC" -> "QUJD".
        let out = data_base64_encoded_data(&mut ctx, data_value(vec![65, 66, 67]), Vec::new())
            .unwrap()
            .result;
        assert_eq!(data_bytes(&out).unwrap(), b"QUJD");
        // The no-options surface rejects extra arguments.
        assert!(data_base64_encoded_data(
            &mut ctx,
            data_value(vec![65]),
            vec![SwiftValue::Bool(true)]
        )
        .is_err());
    }

    #[test]
    fn index_collections_hash_consistently_with_equality() {
        assert_eq!(
            index_path_hash_value(index_path_value(vec![2, 4])).unwrap(),
            index_path_hash_value(index_path_value(vec![2, 4])).unwrap()
        );
        assert_ne!(
            index_path_hash_value(index_path_value(vec![2, 4])).unwrap(),
            index_path_hash_value(index_path_value(vec![2, 5])).unwrap()
        );
        // IndexSets built in different orders normalize to the same set.
        let a: BTreeSet<i128> = [9, 3, 1].into_iter().collect();
        let b: BTreeSet<i128> = [1, 9, 3].into_iter().collect();
        assert_eq!(
            index_set_hash_value(index_set_value(a)).unwrap(),
            index_set_hash_value(index_set_value(b)).unwrap()
        );
    }

    #[test]
    fn equal_data_and_uuid_values_hash_equally() {
        let a = data_value(vec![1, 2, 3]);
        let b = data_value(vec![1, 2, 3]);
        assert_eq!(data_hash_value(a).unwrap(), data_hash_value(b).unwrap());
        let c = data_value(vec![1, 2, 4]);
        assert_ne!(
            data_hash_value(data_value(vec![1, 2, 3])).unwrap(),
            data_hash_value(c).unwrap()
        );
        // Case-insensitive UUID strings normalize to the same value and hash.
        let mut ctx = MockContext::new(0.0);
        let lower = uuid_init(
            &mut ctx,
            vec![Arg {
                label: Some("uuidString".into()),
                value: SwiftValue::Str("e2b8be3f-4c7d-41f3-8d5f-b8d43c343111".into()),
            }],
        )
        .unwrap();
        let upper = uuid_init(
            &mut ctx,
            vec![Arg {
                label: Some("uuidString".into()),
                value: SwiftValue::Str("E2B8BE3F-4C7D-41F3-8D5F-B8D43C343111".into()),
            }],
        )
        .unwrap();
        assert_eq!(
            uuid_hash_value(lower).unwrap(),
            uuid_hash_value(upper).unwrap()
        );
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
        use tswift_core::base64;
        for case in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
            let encoded = base64::encode(case.as_bytes());
            let decoded = base64::decode(&encoded).expect("decodes");
            assert_eq!(decoded, case.as_bytes(), "round-trip for {case:?}");
        }
        assert_eq!(base64::encode(b"Hi"), "SGk=");
        assert_eq!(base64::decode("SGk=").unwrap(), b"Hi");
        // Malformed inputs reject.
        assert!(base64::decode("SGk").is_none()); // wrong length
        assert!(base64::decode("@@@@").is_none()); // bad alphabet
        assert!(base64::decode("====").is_none()); // all padding
        assert!(base64::decode("AA==AAAA").is_none()); // padding before final chunk
        assert!(base64::decode("A===").is_none()); // 3 padding chars
                                                   // Empty decodes to empty Data, matching Foundation.
        assert_eq!(base64::decode("").unwrap(), Vec::<u8>::new());
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
