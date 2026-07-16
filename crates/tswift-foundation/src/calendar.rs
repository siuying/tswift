//! `Calendar` — Gregorian/UTC date arithmetic.
//!
//! The runtime models a single calendar: the proleptic Gregorian calendar in
//! UTC. Date ⇄ component conversion uses Howard Hinnant's `days_from_civil` /
//! `civil_from_days` algorithms (no external crates, offline build). This
//! diverges from Darwin, which honours locale and time zone; that gap is
//! intentional for now.

use std::rc::Rc;

use tswift_core::{
    Arg, BuiltinReceiver, EnumObj, Interpreter, IntrinsicFn, LabeledIntrinsicFn,
    LabeledMethodEntry, MethodEntry, Outcome, PropertyFn, StdContext, StdError, StdResult,
    StructObj, SwiftValue,
};

use crate::{
    calendar_component_name, date_components_value_struct, date_seconds, date_value, type_error,
    DATE_COMPONENT_FIELDS, REFERENCE_DATE_UNIX_OFFSET,
};

const SECONDS_PER_DAY: f64 = 86_400.0;

/// `Calendar.Component` enum cases, in canonical order.
pub(crate) const CALENDAR_COMPONENTS: &[&str] = &[
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
    "dayOfYear",
];

pub fn install(interp: &mut Interpreter<'_>) {
    interp.register_builtin_enum("Calendar.Component", CALENDAR_COMPONENTS);
    interp.register_builtin_enum("Calendar.Identifier", &["gregorian"]);
    interp.register_builtin_enum(
        "Calendar.MatchingPolicy",
        &[
            "nextTime",
            "nextTimePreservingSmallerComponents",
            "previousTimePreservingSmallerComponents",
            "strict",
        ],
    );
    interp.register_builtin_enum("Calendar.SearchDirection", &["forward", "backward"]);
    interp.register_builtin_enum(
        "ComparisonResult",
        &["orderedAscending", "orderedSame", "orderedDescending"],
    );

    interp.register_free_fn("Calendar", calendar_init);
    interp.register_free_fn("TimeZone", timezone_init);
    interp.register_static(BuiltinReceiver::Calendar, "current", calendar_current);
    // Under this runtime's fixed Gregorian/UTC/en_US model there is no
    // locale/timezone tracking, so `autoupdatingCurrent` is identical to
    // `current` (an alias). See frameworks/foundation/scope.toml.
    interp.register_static(
        BuiltinReceiver::Calendar,
        "autoupdatingCurrent",
        calendar_current,
    );
    interp.register_property(BuiltinReceiver::Calendar, "identifier", calendar_identifier);

    // en_US Gregorian symbol tables (locale-independent in this runtime; see
    // `frameworks/foundation/scope.toml`). Standalone variants match the
    // formatting variants in English.
    for (name, func) in [
        ("monthSymbols", months_long as PropertyFn),
        ("standaloneMonthSymbols", months_long),
        ("shortMonthSymbols", months_short),
        ("shortStandaloneMonthSymbols", months_short),
        ("veryShortMonthSymbols", months_very_short),
        ("veryShortStandaloneMonthSymbols", months_very_short),
        ("weekdaySymbols", weekdays_long),
        ("standaloneWeekdaySymbols", weekdays_long),
        ("shortWeekdaySymbols", weekdays_short),
        ("shortStandaloneWeekdaySymbols", weekdays_short),
        ("veryShortWeekdaySymbols", weekdays_very_short),
        ("veryShortStandaloneWeekdaySymbols", weekdays_very_short),
        ("quarterSymbols", quarters_long),
        ("standaloneQuarterSymbols", quarters_long),
        ("shortQuarterSymbols", quarters_short),
        ("shortStandaloneQuarterSymbols", quarters_short),
        ("eraSymbols", eras_short),
        ("longEraSymbols", eras_long),
    ] {
        interp.register_property(BuiltinReceiver::Calendar, name, func);
    }
    interp.register_property(BuiltinReceiver::Calendar, "amSymbol", am_symbol);
    interp.register_property(BuiltinReceiver::Calendar, "pmSymbol", pm_symbol);
    interp.register_property(BuiltinReceiver::Calendar, "firstWeekday", first_weekday);
    interp.register_property(BuiltinReceiver::Calendar, "hashValue", calendar_hash_value);
    interp.register_property(
        BuiltinReceiver::Calendar,
        "minimumDaysInFirstWeek",
        minimum_days_in_first_week,
    );
    interp.register_property(BuiltinReceiver::Calendar, "locale", calendar_locale);
    interp.register_property(BuiltinReceiver::Calendar, "timeZone", calendar_time_zone);
    interp.register_property(
        BuiltinReceiver::Calendar,
        "description",
        calendar_description,
    );
    interp.register_property(
        BuiltinReceiver::Calendar,
        "debugDescription",
        calendar_description,
    );

    for (name, mutating, func) in [
        (
            "dateComponents",
            false,
            calendar_date_components as IntrinsicFn,
        ),
        ("component", false, calendar_component),
        ("startOfDay", false, calendar_start_of_day),
        ("isDate", false, calendar_is_date_in_same_day),
        ("isDateInWeekend", false, calendar_is_date_in_weekend),
        ("isDateInToday", false, calendar_is_date_in_today),
        ("isDateInYesterday", false, calendar_is_date_in_yesterday),
        ("isDateInTomorrow", false, calendar_is_date_in_tomorrow),
        ("compare", false, calendar_compare),
        ("minimumRange", false, calendar_minimum_range),
        ("maximumRange", false, calendar_maximum_range),
    ] {
        interp.register_intrinsic(
            BuiltinReceiver::Calendar,
            name,
            MethodEntry { mutating, func },
        );
    }

    // `date(...)` is overloaded (`from:`, `byAdding:to:`, `byAdding:value:to:`)
    // and must be selected on argument labels.
    interp.register_labeled_intrinsic(
        BuiltinReceiver::Calendar,
        "date",
        LabeledMethodEntry {
            mutating: false,
            func: calendar_date,
        },
    );
    for (name, func) in [
        ("dateInterval", calendar_dateinterval as LabeledIntrinsicFn),
        ("range", calendar_range_of),
        ("ordinality", calendar_ordinality),
        ("nextDate", calendar_next_date),
        ("nextWeekend", calendar_next_weekend),
        ("dateIntervalOfWeekend", calendar_date_interval_of_weekend),
    ] {
        interp.register_labeled_intrinsic(
            BuiltinReceiver::Calendar,
            name,
            LabeledMethodEntry {
                mutating: false,
                func,
            },
        );
    }
}

// ---------------------------------------------------------------------------
// Civil-date math (Howard Hinnant, http://howardhinnant.github.io/date_algorithms.html)
// ---------------------------------------------------------------------------

/// Days since 1970-01-01 for a proleptic-Gregorian y/m/d.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Proleptic-Gregorian y/m/d for a day count since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + i64::from(m <= 2), m, d)
}

pub(crate) fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub(crate) fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(y) => 29,
        2 => 28,
        _ => 30,
    }
}

/// Floor division for `f64` day counts (handles negative reference seconds).
fn floor_div_day(unix_seconds: f64) -> (i64, f64) {
    let days = (unix_seconds / SECONDS_PER_DAY).floor();
    let secs = unix_seconds - days * SECONDS_PER_DAY;
    (days as i64, secs)
}

/// A decomposed UTC instant.
#[derive(Clone, Copy)]
pub(crate) struct Civil {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: i64,
    pub minute: i64,
    pub second: i64,
    pub weekday: i64,
}

pub(crate) fn decompose(ref_seconds: f64) -> Civil {
    let unix = ref_seconds + REFERENCE_DATE_UNIX_OFFSET;
    let (days, secs_in_day) = floor_div_day(unix);
    let (year, month, day) = civil_from_days(days);
    let secs = secs_in_day as i64;
    let weekday = weekday_of_day(days);
    Civil {
        year,
        month,
        day,
        hour: secs / 3600,
        minute: (secs % 3600) / 60,
        second: secs % 60,
        weekday,
    }
}

pub(crate) fn ref_seconds_from_ymdhms(y: i64, m: i64, d: i64, h: i64, min: i64, s: i64) -> f64 {
    let days = days_from_civil(y, m, d);
    let unix = days as f64 * SECONDS_PER_DAY + (h * 3600 + min * 60 + s) as f64;
    unix - REFERENCE_DATE_UNIX_OFFSET
}

// ---------------------------------------------------------------------------
// Builtin wiring
// ---------------------------------------------------------------------------

/// Build a `TimeZone` struct value.  `id` is the normalized IANA identifier
/// (e.g. `"GMT"`); `desc` is the Foundation-style description string.
pub(crate) fn timezone_value(id: &str, desc: &str) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "TimeZone".into(),
        fields: vec![
            ("identifier".into(), SwiftValue::Str(id.into())),
            ("description".into(), SwiftValue::Str(desc.into())),
        ],
    }))
}

/// Normalize a timezone identifier the way Foundation does on Darwin:
/// `"UTC"` → `"GMT"` (they are synonymous; Darwin always returns `"GMT"`).
fn normalize_tz_id(id: &str) -> &str {
    match id {
        "UTC" => "GMT",
        other => other,
    }
}

/// Build a `Locale` struct value with the given identifier.
fn locale_value(id: &str) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "Locale".into(),
        fields: vec![("identifier".into(), SwiftValue::Str(id.into()))],
    }))
}

fn calendar_value(identifier: &str) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "Calendar".into(),
        fields: vec![
            ("_identifier".into(), SwiftValue::Str(identifier.into())),
            // Empty locale matches Darwin's `Calendar(identifier: .gregorian).locale?.identifier`.
            ("locale".into(), locale_value("")),
            // Fixed GMT timezone — the runtime models Gregorian/UTC only.
            ("timeZone".into(), timezone_value("GMT", "GMT")),
        ],
    }))
}

fn calendar_identifier_value(value: &SwiftValue) -> Result<String, StdError> {
    match value {
        SwiftValue::Struct(obj) if obj.type_name == "Calendar" => match obj.get("_identifier") {
            Some(SwiftValue::Str(id)) => Ok(id.to_string()),
            _ => Ok("gregorian".into()),
        },
        other => Err(type_error(format!(
            "expected Calendar, got {}",
            other.type_name()
        ))),
    }
}

fn identifier_arg(value: &SwiftValue) -> Result<String, StdError> {
    match value {
        SwiftValue::Enum(obj) => Ok(obj.case.clone()),
        SwiftValue::Str(name) => Ok(name.to_string()),
        other => Err(type_error(format!(
            "Calendar(identifier:) expects Calendar.Identifier, got {}",
            other.type_name()
        ))),
    }
}

fn calendar_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    match args.as_slice() {
        [arg] if arg.label.as_deref() == Some("identifier") => {
            let id = identifier_arg(&arg.value)?;
            if id != "gregorian" {
                return Err(type_error(format!(
                    "Calendar identifier `{id}` is not supported (only .gregorian)"
                )));
            }
            Ok(calendar_value(&id))
        }
        _ => Err(type_error("Calendar(identifier:) expects one argument")),
    }
}

fn calendar_current(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if !args.is_empty() {
        return Err(type_error("Calendar.current takes no arguments"));
    }
    Ok(calendar_value("gregorian"))
}

fn calendar_identifier(recv: SwiftValue) -> StdResult {
    let id = calendar_identifier_value(&recv)?;
    Ok(SwiftValue::Enum(Rc::new(tswift_core::EnumObj {
        type_name: "Calendar.Identifier".into(),
        case: id,
        payload: Vec::new(),
    })))
}

const MONTHS_LONG: &[&str] = &[
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const MONTHS_SHORT: &[&str] = &[
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTHS_VERY_SHORT: &[&str] = &["J", "F", "M", "A", "M", "J", "J", "A", "S", "O", "N", "D"];
const WEEKDAYS_LONG: &[&str] = &[
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const WEEKDAYS_SHORT: &[&str] = &["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const WEEKDAYS_VERY_SHORT: &[&str] = &["S", "M", "T", "W", "T", "F", "S"];
const QUARTERS_LONG: &[&str] = &["1st quarter", "2nd quarter", "3rd quarter", "4th quarter"];
const QUARTERS_SHORT: &[&str] = &["Q1", "Q2", "Q3", "Q4"];
const ERAS_SHORT: &[&str] = &["BC", "AD"];
const ERAS_LONG: &[&str] = &["Before Christ", "Anno Domini"];

/// Build a `[String]` from a symbol table after validating the receiver is a
/// Gregorian `Calendar`.
fn calendar_symbol_list(recv: &SwiftValue, symbols: &[&str]) -> StdResult {
    calendar_identifier_value(recv)?;
    let values = symbols
        .iter()
        .map(|s| SwiftValue::Str((*s).into()))
        .collect();
    Ok(SwiftValue::Array(Rc::new(values)))
}

fn calendar_scalar_symbol(recv: &SwiftValue, symbol: &str) -> StdResult {
    calendar_identifier_value(recv)?;
    Ok(SwiftValue::Str(symbol.into()))
}

fn months_long(recv: SwiftValue) -> StdResult {
    calendar_symbol_list(&recv, MONTHS_LONG)
}
fn months_short(recv: SwiftValue) -> StdResult {
    calendar_symbol_list(&recv, MONTHS_SHORT)
}
fn months_very_short(recv: SwiftValue) -> StdResult {
    calendar_symbol_list(&recv, MONTHS_VERY_SHORT)
}
fn weekdays_long(recv: SwiftValue) -> StdResult {
    calendar_symbol_list(&recv, WEEKDAYS_LONG)
}
fn weekdays_short(recv: SwiftValue) -> StdResult {
    calendar_symbol_list(&recv, WEEKDAYS_SHORT)
}
fn weekdays_very_short(recv: SwiftValue) -> StdResult {
    calendar_symbol_list(&recv, WEEKDAYS_VERY_SHORT)
}
fn quarters_long(recv: SwiftValue) -> StdResult {
    calendar_symbol_list(&recv, QUARTERS_LONG)
}
fn quarters_short(recv: SwiftValue) -> StdResult {
    calendar_symbol_list(&recv, QUARTERS_SHORT)
}
fn eras_short(recv: SwiftValue) -> StdResult {
    calendar_symbol_list(&recv, ERAS_SHORT)
}
fn eras_long(recv: SwiftValue) -> StdResult {
    calendar_symbol_list(&recv, ERAS_LONG)
}
fn am_symbol(recv: SwiftValue) -> StdResult {
    calendar_scalar_symbol(&recv, "AM")
}
fn pm_symbol(recv: SwiftValue) -> StdResult {
    calendar_scalar_symbol(&recv, "PM")
}
fn first_weekday(recv: SwiftValue) -> StdResult {
    calendar_identifier_value(&recv)?;
    Ok(SwiftValue::int(1))
}
fn calendar_hash_value(recv: SwiftValue) -> StdResult {
    // The runtime models a single calendar, so equal (Gregorian) calendars all
    // hash to the same value derived from their identifier.
    let id = calendar_identifier_value(&recv)?;
    Ok(SwiftValue::int(crate::fnv1a_hash(id.as_bytes())))
}
fn minimum_days_in_first_week(recv: SwiftValue) -> StdResult {
    calendar_identifier_value(&recv)?;
    Ok(SwiftValue::int(1))
}

/// Read an optional Int component out of a DateComponents struct.
fn component_int(obj: &Rc<StructObj>, field: &str) -> Option<i64> {
    match obj.get(field) {
        Some(SwiftValue::Int(i)) => Some(i.raw as i64),
        _ => None,
    }
}

fn date_components_struct(value: &SwiftValue) -> Result<&Rc<StructObj>, StdError> {
    match value {
        SwiftValue::Struct(obj) if obj.type_name == "DateComponents" => Ok(obj),
        other => Err(type_error(format!(
            "expected DateComponents, got {}",
            other.type_name()
        ))),
    }
}

/// Label-aware dispatcher for the `date(...)` overloads.
fn calendar_date(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let labels: Vec<Option<&str>> = args.iter().map(|a| a.label.as_deref()).collect();
    let outcome = match labels.as_slice() {
        [Some("from")] => calendar_date_from(recv, &args[0].value)?,
        [Some("byAdding"), Some("to")] => {
            calendar_date_by_adding_components(recv, &args[0].value, &args[1].value)?
        }
        [Some("byAdding"), Some("value"), Some("to")] => {
            calendar_date_by_adding_component(recv, &args[0].value, &args[1].value, &args[2].value)?
        }
        _ => return Ok(None),
    };
    Ok(Some(outcome))
}

/// `date(from:) -> Date?`. Missing y/m/d default to 1, time fields to 0.
fn calendar_date_from(recv: SwiftValue, components: &SwiftValue) -> Result<Outcome, StdError> {
    let obj = date_components_struct(components)?;
    let year = component_int(obj, "year").unwrap_or(1);
    let month = component_int(obj, "month").unwrap_or(1);
    let day = component_int(obj, "day").unwrap_or(1);
    let hour = component_int(obj, "hour").unwrap_or(0);
    let minute = component_int(obj, "minute").unwrap_or(0);
    let second = component_int(obj, "second").unwrap_or(0);
    let seconds = ref_seconds_from_ymdhms(year, month, day, hour, minute, second);
    Ok(Outcome {
        result: date_value(seconds),
        receiver: recv,
    })
}

/// Add a y/m/d/h/m/s delta to `base`, normalising month/day overflow.
fn add_delta(base: f64, delta: &Delta) -> f64 {
    let civil = decompose(base);
    // Year + month carry, then clamp the day into the resulting month.
    let mut year = civil.year + delta.year;
    let mut month0 = civil.month - 1 + delta.month;
    year += month0.div_euclid(12);
    month0 = month0.rem_euclid(12);
    let month = month0 + 1;
    let day = civil.day.min(days_in_month(year, month));
    let normalized =
        ref_seconds_from_ymdhms(year, month, day, civil.hour, civil.minute, civil.second);
    // Day/week/time deltas are plain second offsets.
    normalized
        + (delta.day + delta.week * 7) as f64 * SECONDS_PER_DAY
        + (delta.hour * 3600 + delta.minute * 60 + delta.second) as f64
}

#[derive(Default)]
struct Delta {
    year: i64,
    month: i64,
    day: i64,
    week: i64,
    hour: i64,
    minute: i64,
    second: i64,
}

fn calendar_date_by_adding_components(
    recv: SwiftValue,
    components: &SwiftValue,
    date: &SwiftValue,
) -> Result<Outcome, StdError> {
    let obj = date_components_struct(components)?;
    let delta = Delta {
        year: component_int(obj, "year").unwrap_or(0),
        month: component_int(obj, "month").unwrap_or(0),
        day: component_int(obj, "day").unwrap_or(0),
        week: component_int(obj, "weekOfYear").unwrap_or(0),
        hour: component_int(obj, "hour").unwrap_or(0),
        minute: component_int(obj, "minute").unwrap_or(0),
        second: component_int(obj, "second").unwrap_or(0),
    };
    let result = add_delta(date_seconds(date)?, &delta);
    Ok(Outcome {
        result: date_value(result),
        receiver: recv,
    })
}

fn calendar_date_by_adding_component(
    recv: SwiftValue,
    component: &SwiftValue,
    value: &SwiftValue,
    date: &SwiftValue,
) -> Result<Outcome, StdError> {
    let name = calendar_component_name(component)?;
    let amount = match value {
        SwiftValue::Int(i) => i.raw as i64,
        other => {
            return Err(type_error(format!(
                "Calendar.date(byAdding:value:to:) expects Int value, got {}",
                other.type_name()
            )))
        }
    };
    let mut delta = Delta::default();
    match name.as_str() {
        "year" => delta.year = amount,
        "month" => delta.month = amount,
        "day" => delta.day = amount,
        "weekOfYear" | "weekOfMonth" => delta.week = amount,
        "hour" => delta.hour = amount,
        "minute" => delta.minute = amount,
        "second" => delta.second = amount,
        other => {
            return Err(type_error(format!(
                "Calendar.date(byAdding:) does not support component `{other}`"
            )))
        }
    }
    let result = add_delta(date_seconds(date)?, &delta);
    Ok(Outcome {
        result: date_value(result),
        receiver: recv,
    })
}

/// The requested-component name set out of a `Set`/array argument.
fn requested_components(value: &SwiftValue) -> Result<Vec<String>, StdError> {
    let items: Vec<SwiftValue> = match value {
        SwiftValue::Set(set) => set.iter().cloned().collect(),
        SwiftValue::Array(arr) => arr.iter().cloned().collect(),
        other => {
            return Err(type_error(format!(
                "expected Set<Calendar.Component>, got {}",
                other.type_name()
            )))
        }
    };
    items.iter().map(calendar_component_name).collect()
}

fn component_value(civil: &Civil, name: &str) -> Option<i64> {
    match name {
        "era" => Some(1),
        "year" | "yearForWeekOfYear" => Some(civil.year),
        "month" => Some(civil.month),
        "day" => Some(civil.day),
        "hour" => Some(civil.hour),
        "minute" => Some(civil.minute),
        "second" => Some(civil.second),
        "nanosecond" => Some(0),
        "weekday" => Some(civil.weekday),
        "weekdayOrdinal" => Some((civil.day - 1) / 7 + 1),
        "quarter" => Some((civil.month - 1) / 3 + 1),
        "weekOfMonth" => Some((civil.day - 1) / 7 + 1),
        "weekOfYear" => {
            let day_of_year = days_from_civil(civil.year, civil.month, civil.day)
                - days_from_civil(civil.year, 1, 1)
                + 1;
            Some((day_of_year - 1) / 7 + 1)
        }
        "dayOfYear" => Some(
            days_from_civil(civil.year, civil.month, civil.day) - days_from_civil(civil.year, 1, 1)
                + 1,
        ),
        _ => None,
    }
}

fn calendar_date_components(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (set, date) = match args.as_slice() {
        [set, date] => (set, date),
        _ => {
            return Err(type_error(
                "Calendar.dateComponents(_:from:) expects two arguments",
            ))
        }
    };
    let names = requested_components(set)?;
    let civil = decompose(date_seconds(date)?);
    let mut fields: Vec<(String, SwiftValue)> = DATE_COMPONENT_FIELDS
        .iter()
        .map(|name| ((*name).to_string(), SwiftValue::Nil))
        .collect();
    for name in names {
        if let Some(value) = component_value(&civil, &name) {
            if let Some(slot) = fields.iter_mut().find(|(field, _)| field == &name) {
                slot.1 = SwiftValue::int(value as i128);
            }
        }
    }
    Ok(Outcome {
        result: date_components_value_struct(fields),
        receiver: recv,
    })
}

fn calendar_component(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (component, date) = match args.as_slice() {
        [component, date] => (component, date),
        _ => {
            return Err(type_error(
                "Calendar.component(_:from:) expects two arguments",
            ))
        }
    };
    let name = calendar_component_name(component)?;
    let civil = decompose(date_seconds(date)?);
    let value = component_value(&civil, &name).unwrap_or(0);
    Ok(Outcome {
        result: SwiftValue::int(value as i128),
        receiver: recv,
    })
}

fn calendar_start_of_day(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let [date] = args.as_slice() else {
        return Err(type_error("Calendar.startOfDay(for:) expects one argument"));
    };
    let unix = date_seconds(date)? + REFERENCE_DATE_UNIX_OFFSET;
    let (days, _) = floor_div_day(unix);
    let start = days as f64 * SECONDS_PER_DAY - REFERENCE_DATE_UNIX_OFFSET;
    Ok(Outcome {
        result: date_value(start),
        receiver: recv,
    })
}

fn calendar_is_date_in_same_day(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (lhs, rhs) = match args.as_slice() {
        [lhs, rhs] => (lhs, rhs),
        _ => {
            return Err(type_error(
                "Calendar.isDate(_:inSameDayAs:) expects two arguments",
            ))
        }
    };
    let (lday, _) = floor_div_day(date_seconds(lhs)? + REFERENCE_DATE_UNIX_OFFSET);
    let (rday, _) = floor_div_day(date_seconds(rhs)? + REFERENCE_DATE_UNIX_OFFSET);
    Ok(Outcome {
        result: SwiftValue::Bool(lday == rday),
        receiver: recv,
    })
}

/// The UTC day index (days since 1970-01-01) of a `Date` argument.
fn date_day_index(args: &[SwiftValue], method: &str) -> Result<i64, StdError> {
    let [date] = args else {
        return Err(type_error(format!(
            "Calendar.{method} expects one argument"
        )));
    };
    let (day, _) = floor_div_day(date_seconds(date)? + REFERENCE_DATE_UNIX_OFFSET);
    Ok(day)
}

/// The UTC day index of the current instant.
fn today_day_index(ctx: &mut dyn StdContext) -> i64 {
    let (day, _) = floor_div_day(ctx.now_unix_seconds());
    day
}

/// Swift weekday (1=Sunday..7=Saturday) for a day index since 1970-01-01
/// (day 0 is a Thursday).
pub(crate) fn weekday_of_day(day: i64) -> i64 {
    (day.rem_euclid(7) + 4) % 7 + 1
}

/// 1-based day-of-year for a civil date (Jan 1 == 1).
pub(crate) fn day_of_year(civil: &Civil) -> i64 {
    days_from_civil(civil.year, civil.month, civil.day) - days_from_civil(civil.year, 1, 1) + 1
}

/// 1-based calendar quarter (1..=4) for a civil date.
pub(crate) fn quarter_of(civil: &Civil) -> i64 {
    (civil.month - 1) / 3 + 1
}

fn calendar_is_date_in_weekend(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let day = date_day_index(&args, "isDateInWeekend(_:)")?;
    let weekday = weekday_of_day(day);
    // Saturday (7) and Sunday (1) in en_US Gregorian.
    Ok(Outcome {
        result: SwiftValue::Bool(weekday == 1 || weekday == 7),
        receiver: recv,
    })
}

fn calendar_is_date_in_today(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let day = date_day_index(&args, "isDateInToday(_:)")?;
    let today = today_day_index(ctx);
    Ok(Outcome {
        result: SwiftValue::Bool(day == today),
        receiver: recv,
    })
}

fn calendar_is_date_in_yesterday(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let day = date_day_index(&args, "isDateInYesterday(_:)")?;
    let today = today_day_index(ctx);
    Ok(Outcome {
        result: SwiftValue::Bool(day == today - 1),
        receiver: recv,
    })
}

fn calendar_is_date_in_tomorrow(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let day = date_day_index(&args, "isDateInTomorrow(_:)")?;
    let today = today_day_index(ctx);
    Ok(Outcome {
        result: SwiftValue::Bool(day == today + 1),
        receiver: recv,
    })
}

// ---------------------------------------------------------------------------
// New calendar properties
// ---------------------------------------------------------------------------

/// Extract the locale identifier from a stored `locale` field on Calendar.
fn calendar_locale_id(cal: &Rc<StructObj>) -> String {
    match cal.get("locale") {
        Some(SwiftValue::Struct(loc)) => match loc.get("identifier") {
            Some(SwiftValue::Str(s)) => s.clone(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

/// Extract the timezone description from a stored `timeZone` field on Calendar.
fn calendar_tz_desc(cal: &Rc<StructObj>) -> String {
    match cal.get("timeZone") {
        Some(SwiftValue::Struct(tz)) => match tz.get("description") {
            Some(SwiftValue::Str(s)) => s.clone(),
            _ => "GMT".to_string(),
        },
        _ => "GMT".to_string(),
    }
}

/// Foundation-style description of a Calendar struct:
/// `"{id} ({id}) locale: {locale_id} time zone: {tz_desc} firstWeekday: 1 minDaysInFirstWeek: 1"`
pub(crate) fn calendar_description_str(cal: &Rc<StructObj>) -> String {
    let id = match cal.get("_identifier") {
        Some(SwiftValue::Str(s)) => s.clone(),
        _ => "gregorian".to_string(),
    };
    let locale_id = calendar_locale_id(cal);
    let tz_desc = calendar_tz_desc(cal);
    format!("{id} ({id}) locale: {locale_id} time zone: {tz_desc} firstWeekday: 1 minDaysInFirstWeek: 1")
}

fn calendar_locale(recv: SwiftValue) -> StdResult {
    // The stored `locale` field is returned directly if present (the struct
    // field path takes precedence over the registered getter).  This getter
    // exists as a fallback for Calendar values created before the field was
    // added and for callers that go through the property registry.
    let ok = match &recv {
        SwiftValue::Struct(obj) if obj.type_name == "Calendar" => obj
            .get("locale")
            .cloned()
            .unwrap_or_else(|| locale_value("")),
        _ => return Err(type_error("expected Calendar")),
    };
    Ok(ok)
}

fn calendar_time_zone(recv: SwiftValue) -> StdResult {
    // The stored `timeZone` field is returned directly if present.
    let ok = match &recv {
        SwiftValue::Struct(obj) if obj.type_name == "Calendar" => obj
            .get("timeZone")
            .cloned()
            .unwrap_or_else(|| timezone_value("GMT", "GMT")),
        _ => return Err(type_error("expected Calendar")),
    };
    Ok(ok)
}

fn calendar_description(recv: SwiftValue) -> StdResult {
    let SwiftValue::Struct(obj) = &recv else {
        return Err(type_error(format!(
            "expected Calendar, got {}",
            recv.type_name()
        )));
    };
    if obj.type_name != "Calendar" {
        return Err(type_error(format!(
            "expected Calendar, got {}",
            obj.type_name
        )));
    }
    Ok(SwiftValue::Str(calendar_description_str(obj)))
}

// ---------------------------------------------------------------------------
// TimeZone free functions
// ---------------------------------------------------------------------------

/// `TimeZone(identifier:)` and `TimeZone(abbreviation:)` — failable inits.
///
/// `identifier:` normalises `"UTC"` → `"GMT"`; returns `nil` for other
/// unknown identifiers.  `abbreviation:` maps `"GMT"`/`"UTC"` to a timezone
/// whose `description` is `"GMT (0)"` (matching Darwin's abbreviation init).
fn timezone_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let [arg] = args.as_slice() else {
        return Err(type_error(
            "TimeZone expects exactly one argument (identifier: or abbreviation:)",
        ));
    };
    let SwiftValue::Str(val) = &arg.value else {
        return Err(type_error("TimeZone init expects String"));
    };
    match arg.label.as_deref() {
        Some("identifier") => {
            let norm = normalize_tz_id(val);
            if norm == "GMT" {
                Ok(timezone_value("GMT", "GMT"))
            } else {
                Ok(SwiftValue::Nil)
            }
        }
        Some("abbreviation") => match val.as_str() {
            "GMT" | "UTC" => Ok(timezone_value("GMT", "GMT (0)")),
            _ => Ok(SwiftValue::Nil),
        },
        _ => Err(type_error(
            "TimeZone expects `identifier:` or `abbreviation:` label",
        )),
    }
}

// ---------------------------------------------------------------------------
// compare(_:to:toGranularity:)
// ---------------------------------------------------------------------------

/// Truncate a `Civil` value to the given granularity level, returning the
/// y/m/d/h/min/s tuple with sub-granularity fields zeroed (or set to their
/// minimum valid values).
fn truncate_civil(c: &Civil, granularity: &str) -> (i64, i64, i64, i64, i64, i64) {
    match granularity {
        "era" | "year" => (c.year, 1, 1, 0, 0, 0),
        "month" => (c.year, c.month, 1, 0, 0, 0),
        "day" => (c.year, c.month, c.day, 0, 0, 0),
        "hour" => (c.year, c.month, c.day, c.hour, 0, 0),
        "minute" => (c.year, c.month, c.day, c.hour, c.minute, 0),
        // second / nanosecond: include up through seconds
        _ => (c.year, c.month, c.day, c.hour, c.minute, c.second),
    }
}

fn comparison_result_value(case: &str) -> SwiftValue {
    SwiftValue::Enum(Rc::new(EnumObj {
        type_name: "ComparisonResult".into(),
        case: case.into(),
        payload: Vec::new(),
    }))
}

/// `Calendar.compare(_:to:toGranularity:) -> ComparisonResult`.
///
/// Truncates both dates to the given `Calendar.Component` granularity and
/// returns `.orderedAscending`, `.orderedSame`, or `.orderedDescending`.
fn calendar_compare(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let (date1, date2, component) = match args.as_slice() {
        [d1, d2, c] => (d1, d2, c),
        _ => {
            return Err(type_error(
                "Calendar.compare(_:to:toGranularity:) expects three arguments",
            ))
        }
    };
    let gran = calendar_component_name(component)?;
    let c1 = decompose(date_seconds(date1)?);
    let c2 = decompose(date_seconds(date2)?);
    let (y1, m1, d1, h1, min1, s1) = truncate_civil(&c1, &gran);
    let (y2, m2, d2, h2, min2, s2) = truncate_civil(&c2, &gran);
    let t1 = ref_seconds_from_ymdhms(y1, m1, d1, h1, min1, s1);
    let t2 = ref_seconds_from_ymdhms(y2, m2, d2, h2, min2, s2);
    let case = if t1 < t2 {
        "orderedAscending"
    } else if t1 > t2 {
        "orderedDescending"
    } else {
        "orderedSame"
    };
    Ok(Outcome {
        result: comparison_result_value(case),
        receiver: recv,
    })
}

// ---------------------------------------------------------------------------
// dateInterval(of:for:)
// ---------------------------------------------------------------------------

/// Build a `DateInterval` struct value with `start: Date` and `duration: Double`.
fn date_interval_value(start: f64, duration: f64) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "DateInterval".into(),
        fields: vec![
            ("start".into(), date_value(start)),
            ("duration".into(), SwiftValue::Double(duration)),
        ],
    }))
}

/// `Calendar.dateInterval(of:for:) -> DateInterval?`.
///
/// Returns the `DateInterval` for the calendar unit containing `date`.
/// Supported components: `.day`, `.month`, `.year`, `.weekOfYear`, `.hour`.
/// Returns `nil` for unsupported components.
fn calendar_dateinterval(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let labels: Vec<Option<&str>> = args.iter().map(|a| a.label.as_deref()).collect();
    // Only handle the `dateInterval(of:for:)` variant.
    let (component_val, date_val) = match labels.as_slice() {
        [Some("of"), Some("for")] => (&args[0].value, &args[1].value),
        _ => return Ok(None),
    };
    let gran = calendar_component_name(component_val)?;
    let ref_secs = date_seconds(date_val)?;
    let unix = ref_secs + REFERENCE_DATE_UNIX_OFFSET;
    let (day_num, secs_in_day) = floor_div_day(unix);
    let civil = decompose(ref_secs);

    let result = match gran.as_str() {
        "day" => {
            // start = midnight of this day; duration = 86400 seconds.
            let start_unix = day_num as f64 * SECONDS_PER_DAY;
            let start = start_unix - REFERENCE_DATE_UNIX_OFFSET;
            Some(date_interval_value(start, SECONDS_PER_DAY))
        }
        "month" => {
            let start = ref_seconds_from_ymdhms(civil.year, civil.month, 1, 0, 0, 0);
            let duration = days_in_month(civil.year, civil.month) as f64 * SECONDS_PER_DAY;
            Some(date_interval_value(start, duration))
        }
        "year" => {
            let start = ref_seconds_from_ymdhms(civil.year, 1, 1, 0, 0, 0);
            let days = if is_leap_year(civil.year) { 366 } else { 365 };
            let duration = days as f64 * SECONDS_PER_DAY;
            Some(date_interval_value(start, duration))
        }
        "weekOfYear" | "weekOfMonth" => {
            // For en_US Gregorian (firstWeekday = Sunday = 1),
            // the week starts on the most-recent Sunday.
            // weekday: 1=Sun, 2=Mon, ..., 7=Sat → days_back = weekday - 1.
            let days_back = civil.weekday - 1;
            let week_start_unix = (day_num - days_back) as f64 * SECONDS_PER_DAY;
            let start = week_start_unix - REFERENCE_DATE_UNIX_OFFSET;
            let duration = 7.0 * SECONDS_PER_DAY;
            Some(date_interval_value(start, duration))
        }
        "hour" => {
            let start =
                ref_seconds_from_ymdhms(civil.year, civil.month, civil.day, civil.hour, 0, 0);
            Some(date_interval_value(start, 3600.0))
        }
        _ => None, // unsupported component → return nil
    };

    // Suppress unused-variable warning in the day-interval path.
    let _ = secs_in_day;

    Ok(Some(Outcome {
        result: result.unwrap_or(SwiftValue::Nil),
        receiver: recv,
    }))
}

// ---------------------------------------------------------------------------
// range(of:in:for:)
// ---------------------------------------------------------------------------

/// `Calendar.range(of:in:for:) -> Range<Int>?`.
///
/// Returns the `Range<Int>` of valid values for the smaller component within
/// the larger component that contains `date`.
fn calendar_range_of(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let labels: Vec<Option<&str>> = args.iter().map(|a| a.label.as_deref()).collect();
    let (smaller_val, larger_val, date_val) = match labels.as_slice() {
        [Some("of"), Some("in"), Some("for")] => (&args[0].value, &args[1].value, &args[2].value),
        _ => return Ok(None),
    };
    let smaller = calendar_component_name(smaller_val)?;
    let larger = calendar_component_name(larger_val)?;
    let civil = decompose(date_seconds(date_val)?);

    let range = range_of_in(&civil, &smaller, &larger);
    Ok(Some(Outcome {
        result: match range {
            Some((lo, hi)) => SwiftValue::Range {
                lo: lo as i128,
                hi: hi as i128,
                inclusive: false,
            },
            None => SwiftValue::Nil,
        },
        receiver: recv,
    }))
}

/// Return `Some((lo, hi))` for the valid range of `smaller` within `larger`
/// as determined by `civil`. `hi` is exclusive (past-the-end).
fn range_of_in(civil: &Civil, smaller: &str, larger: &str) -> Option<(i64, i64)> {
    match (smaller, larger) {
        ("day", "month") => {
            let dim = days_in_month(civil.year, civil.month);
            Some((1, dim + 1))
        }
        ("day", "year") => {
            let days = if is_leap_year(civil.year) { 366 } else { 365 };
            Some((1, days + 1))
        }
        ("month", "year") => Some((1, 13)),
        ("hour", "day") => Some((0, 24)),
        ("minute", "hour") => Some((0, 60)),
        ("second", "minute") => Some((0, 60)),
        ("nanosecond", "second") => Some((0, 1_000_000_000)),
        ("weekday", "weekOfYear") | ("weekday", "weekOfMonth") => Some((1, 8)),
        ("weekOfYear", "year") => {
            // ISO-8601 can reach week 53; Gregorian typically 52-53.
            // Report max possible (53 weeks = 1..<54) as an honest upper bound.
            Some((1, 54))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// minimumRange(of:) / maximumRange(of:)
// ---------------------------------------------------------------------------

/// Gregorian constants for the smallest range a component can occupy in any
/// month/year/etc. (`minimumRange`) or the largest (`maximumRange`).
fn gregorian_range(component: &str, minimum: bool) -> Option<(i64, i64)> {
    // (lo, hi_exclusive)
    let pair = match component {
        "day" => {
            if minimum {
                (1, 29) // February non-leap: 28 days
            } else {
                (1, 32) // January / March / … : 31 days
            }
        }
        "month" => (1, 13),  // always 12 months
        "hour" => (0, 24),   // always 24 hours
        "minute" => (0, 60), // always 60 minutes
        "second" => (0, 60), // ignoring leap seconds
        "nanosecond" => (0, 1_000_000_000),
        "weekday" => (1, 8), // Sun=1 … Sat=7
        "weekOfYear" => {
            if minimum {
                (1, 53) // some years have only 52 weeks
            } else {
                (1, 54) // some years reach week 53
            }
        }
        "weekOfMonth" => {
            if minimum {
                (1, 5)
            } else {
                (1, 7)
            }
        }
        "quarter" => (1, 5), // 4 quarters
        "era" => (0, 2),     // BC (0) and AD (1)
        _ => return None,
    };
    Some(pair)
}

/// `Calendar.minimumRange(of:) -> Range<Int>?`
fn calendar_minimum_range(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let [component] = args.as_slice() else {
        return Err(type_error(
            "Calendar.minimumRange(of:) expects one argument",
        ));
    };
    let name = calendar_component_name(component)?;
    let result = match gregorian_range(&name, true) {
        Some((lo, hi)) => SwiftValue::Range {
            lo: lo as i128,
            hi: hi as i128,
            inclusive: false,
        },
        None => SwiftValue::Nil,
    };
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

/// `Calendar.maximumRange(of:) -> Range<Int>?`
fn calendar_maximum_range(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let [component] = args.as_slice() else {
        return Err(type_error(
            "Calendar.maximumRange(of:) expects one argument",
        ));
    };
    let name = calendar_component_name(component)?;
    let result = match gregorian_range(&name, false) {
        Some((lo, hi)) => SwiftValue::Range {
            lo: lo as i128,
            hi: hi as i128,
            inclusive: false,
        },
        None => SwiftValue::Nil,
    };
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

// ---------------------------------------------------------------------------
// ordinality(of:in:for:)
// ---------------------------------------------------------------------------

/// `Calendar.ordinality(of:in:for:) -> Int?`.
///
/// Returns the 1-based ordinal of `smaller` within `larger` for `date`.
fn calendar_ordinality(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let labels: Vec<Option<&str>> = args.iter().map(|a| a.label.as_deref()).collect();
    let (smaller_val, larger_val, date_val) = match labels.as_slice() {
        [Some("of"), Some("in"), Some("for")] => (&args[0].value, &args[1].value, &args[2].value),
        _ => return Ok(None),
    };
    let smaller = calendar_component_name(smaller_val)?;
    let larger = calendar_component_name(larger_val)?;
    let civil = decompose(date_seconds(date_val)?);

    let ordinal: Option<i64> = match (smaller.as_str(), larger.as_str()) {
        ("day", "year") => Some(
            days_from_civil(civil.year, civil.month, civil.day) - days_from_civil(civil.year, 1, 1)
                + 1,
        ),
        ("day", "month") => Some(civil.day),
        ("month", "year") => Some(civil.month),
        ("weekday", "weekOfYear") | ("weekday", "weekOfMonth") => Some(civil.weekday),
        ("hour", "day") => Some(civil.hour + 1),
        ("minute", "hour") => Some(civil.minute + 1),
        ("second", "minute") => Some(civil.second + 1),
        ("weekOfYear", "year") => {
            let day_of_year = days_from_civil(civil.year, civil.month, civil.day)
                - days_from_civil(civil.year, 1, 1)
                + 1;
            Some((day_of_year - 1) / 7 + 1)
        }
        _ => None,
    };

    Ok(Some(Outcome {
        result: ordinal
            .map(|o| SwiftValue::int(o as i128))
            .unwrap_or(SwiftValue::Nil),
        receiver: recv,
    }))
}

// ---------------------------------------------------------------------------
// nextDate(after:matching:matchingPolicy:)
// ---------------------------------------------------------------------------

/// Returns `true` when all non-nil component fields of `matching` are satisfied
/// by the decomposed date `c`.
fn matches_components(c: &Civil, matching: &Rc<StructObj>) -> bool {
    let check = |field: &str, actual: i64| -> bool {
        match component_int(matching, field) {
            Some(expected) => expected == actual,
            None => true, // field not specified → always matches
        }
    };
    check("year", c.year)
        && check("month", c.month)
        && check("day", c.day)
        && check("hour", c.hour)
        && check("minute", c.minute)
        && check("second", c.second)
        && check("weekday", c.weekday)
}

/// Whether `matching` specifies any time-of-day component (hour/minute/
/// second/nanosecond).
fn has_time_match_components(matching: &Rc<StructObj>) -> bool {
    ["hour", "minute", "second", "nanosecond"]
        .iter()
        .any(|f| component_int(matching, f).is_some())
}

/// Whether `matching` specifies any date-level component (weekday, day, month,
/// year, weekOfYear, …).
fn has_date_match_components(matching: &Rc<StructObj>) -> bool {
    [
        "year",
        "month",
        "day",
        "weekday",
        "weekdayOrdinal",
        "weekOfYear",
        "weekOfMonth",
        "quarter",
    ]
    .iter()
    .any(|f| component_int(matching, f).is_some())
}

/// `Calendar.nextDate(after:matching:matchingPolicy:) -> Date?`.
///
/// Finds the chronologically first date strictly after `after` whose
/// decomposition satisfies every specified component of `matching`.
///
/// **Supported policy**: `.nextTime` only. Other policies return an error
/// (honest subset — see notes.md).
///
/// **Foundation semantics for date-only matching** (e.g. `weekday:`): when no
/// time components are specified, unspecified smaller components default to
/// their minimum (0), so the result is snapped to midnight of the matching
/// day, not the time-of-day from `after`.
fn calendar_next_date(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let labels: Vec<Option<&str>> = args.iter().map(|a| a.label.as_deref()).collect();
    let (after_val, matching_val, policy_val) = match labels.as_slice() {
        [Some("after"), Some("matching"), Some("matchingPolicy")] => {
            (&args[0].value, &args[1].value, &args[2].value)
        }
        _ => return Ok(None),
    };

    // Policy guard: only .nextTime is implemented.
    let policy = calendar_component_name(policy_val)?;
    if policy != "nextTime" {
        return Err(type_error(format!(
            "Calendar.nextDate: matchingPolicy .{policy} is not supported; \
             only .nextTime is implemented in this runtime"
        )));
    }

    let after = date_seconds(after_val)?;
    let obj = date_components_struct(matching_val)?;

    let has_date = has_date_match_components(obj);
    let has_time = has_time_match_components(obj);

    let result = if has_date && !has_time {
        // Date-only matching (e.g. weekday:, day:, month:).
        //
        // Foundation's .nextTime semantics: unspecified smaller components
        // (hour, minute, second) default to their minimum (0), so the result
        // is always midnight UTC of the first matching day strictly after `after`.
        let unix_after = after + REFERENCE_DATE_UNIX_OFFSET;
        let (after_day, _) = floor_div_day(unix_after);
        // Start from midnight of the next calendar day (always > after).
        let mut found = SwiftValue::Nil;
        for day in (after_day + 1..).take(400) {
            let midnight_ref = (day as f64 * SECONDS_PER_DAY) - REFERENCE_DATE_UNIX_OFFSET;
            let civil = decompose(midnight_ref);
            if matches_components(&civil, obj) {
                found = date_value(midnight_ref);
                break;
            }
        }
        found
    } else if !has_date {
        // Time-only matching (e.g. hour:, minute:): step 1 second.
        let mut t = after + 1.0;
        let mut found = SwiftValue::Nil;
        for _ in 0..(2 * 86_400usize) {
            let civil = decompose(t);
            if matches_components(&civil, obj) {
                found = date_value(t);
                break;
            }
            t += 1.0;
        }
        found
    } else {
        // Mixed date+time: step 1 second, cap at 35 days.
        let mut t = after + 1.0;
        let mut found = SwiftValue::Nil;
        for _ in 0..(35 * 86_400usize) {
            let civil = decompose(t);
            if matches_components(&civil, obj) {
                found = date_value(t);
                break;
            }
            t += 1.0;
        }
        found
    };

    Ok(Some(Outcome {
        result,
        receiver: recv,
    }))
}

// ---------------------------------------------------------------------------
// nextWeekend / dateIntervalOfWeekend
// ---------------------------------------------------------------------------
//
// "Weekend" here is Saturday + Sunday, the Gregorian convention under this
// runtime's fixed en_US/Gregorian stance. Darwin's real behaviour is
// Locale-dependent (some locales use Fri/Sat); documented in scope.toml.

const WEEKEND_DURATION: f64 = 2.0 * SECONDS_PER_DAY;

/// Ref-seconds of midnight (UTC) for a day index since 1970-01-01.
fn midnight_ref(day: i64) -> f64 {
    day as f64 * SECONDS_PER_DAY - REFERENCE_DATE_UNIX_OFFSET
}

/// `Calendar.nextWeekend(startingAfter:) -> DateInterval?`.
///
/// Returns the Saturday 00:00 .. Monday 00:00 span whose start is the first
/// Saturday-midnight strictly after `date`. (Under a fixed Gregorian calendar
/// this never returns nil; the optional matches Darwin's signature.)
fn calendar_next_weekend(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let labels: Vec<Option<&str>> = args.iter().map(|a| a.label.as_deref()).collect();
    let date_val = match labels.as_slice() {
        [Some("startingAfter")] => &args[0].value,
        _ => return Ok(None),
    };
    let after = date_seconds(date_val)?;
    let unix_after = after + REFERENCE_DATE_UNIX_OFFSET;
    let (after_day, _) = floor_div_day(unix_after);
    // First Saturday (weekday 7) midnight strictly after `date`.
    let mut result = SwiftValue::Nil;
    for day in after_day..(after_day + 8) {
        if weekday_of_day(day) == 7 {
            let start = midnight_ref(day);
            if start > after {
                result = date_interval_value(start, WEEKEND_DURATION);
                break;
            }
        }
    }
    Ok(Some(Outcome {
        result,
        receiver: recv,
    }))
}

/// `Calendar.dateIntervalOfWeekend(containing:) -> DateInterval?`.
///
/// If `date` falls on a Saturday or Sunday, returns the enclosing
/// Saturday 00:00 .. Monday 00:00 interval; otherwise nil. Monday 00:00 is the
/// exclusive end, so it is *not* considered part of the weekend.
fn calendar_date_interval_of_weekend(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let labels: Vec<Option<&str>> = args.iter().map(|a| a.label.as_deref()).collect();
    let date_val = match labels.as_slice() {
        [Some("containing")] => &args[0].value,
        _ => return Ok(None),
    };
    let ref_secs = date_seconds(date_val)?;
    let (day, _) = floor_div_day(ref_secs + REFERENCE_DATE_UNIX_OFFSET);
    let weekday = weekday_of_day(day);
    // Saturday = 7, Sunday = 1. The weekend starts on the most-recent Saturday.
    let result = match weekday {
        7 => date_interval_value(midnight_ref(day), WEEKEND_DURATION),
        1 => date_interval_value(midnight_ref(day - 1), WEEKEND_DURATION),
        _ => SwiftValue::Nil,
    };
    Ok(Some(Outcome {
        result,
        receiver: recv,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trips_through_days() {
        for &(y, m, d) in &[(2024, 2, 29), (2023, 12, 31), (1970, 1, 1), (1, 1, 1)] {
            let days = days_from_civil(y, m, d);
            assert_eq!(civil_from_days(days), (y, m, d));
        }
    }

    #[test]
    fn leap_year_february_has_29_days() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
    }

    #[test]
    fn decompose_recovers_components() {
        // 2024-06-29 09:41:00 UTC.
        let seconds = ref_seconds_from_ymdhms(2024, 6, 29, 9, 41, 0);
        let civil = decompose(seconds);
        assert_eq!((civil.year, civil.month, civil.day), (2024, 6, 29));
        assert_eq!((civil.hour, civil.minute, civil.second), (9, 41, 0));
        // 2024-06-29 is a Saturday → Swift weekday 7.
        assert_eq!(civil.weekday, 7);
    }

    fn gregorian() -> SwiftValue {
        calendar_value("gregorian")
    }

    fn as_strings(value: SwiftValue) -> Vec<String> {
        match value {
            SwiftValue::Array(items) => items
                .iter()
                .map(|v| match v {
                    SwiftValue::Str(s) => s.to_string(),
                    other => panic!("expected String symbol, got {other:?}"),
                })
                .collect(),
            other => panic!("expected [String], got {other:?}"),
        }
    }

    #[test]
    fn month_symbols_are_twelve_english_names() {
        let long = as_strings(months_long(gregorian()).unwrap());
        assert_eq!(long.len(), 12);
        assert_eq!(long[0], "January");
        assert_eq!(long[11], "December");
        assert_eq!(as_strings(months_short(gregorian()).unwrap())[2], "Mar");
        assert_eq!(as_strings(months_very_short(gregorian()).unwrap())[4], "M");
    }

    #[test]
    fn weekday_symbols_start_on_sunday() {
        let long = as_strings(weekdays_long(gregorian()).unwrap());
        assert_eq!(long.first().map(String::as_str), Some("Sunday"));
        assert_eq!(long.last().map(String::as_str), Some("Saturday"));
        assert_eq!(as_strings(weekdays_short(gregorian()).unwrap())[6], "Sat");
    }

    #[test]
    fn quarter_and_era_symbols() {
        assert_eq!(
            as_strings(quarters_short(gregorian()).unwrap()),
            ["Q1", "Q2", "Q3", "Q4"]
        );
        assert_eq!(as_strings(eras_short(gregorian()).unwrap()), ["BC", "AD"]);
        assert_eq!(
            as_strings(eras_long(gregorian()).unwrap()),
            ["Before Christ", "Anno Domini"]
        );
    }

    #[test]
    fn calendar_hash_is_stable_for_gregorian() {
        assert_eq!(
            calendar_hash_value(gregorian()).unwrap(),
            calendar_hash_value(gregorian()).unwrap()
        );
    }

    #[test]
    fn scalar_symbols_and_week_settings() {
        assert_eq!(
            am_symbol(gregorian()).unwrap(),
            SwiftValue::Str("AM".into())
        );
        assert_eq!(
            pm_symbol(gregorian()).unwrap(),
            SwiftValue::Str("PM".into())
        );
        assert_eq!(first_weekday(gregorian()).unwrap(), SwiftValue::int(1));
        assert_eq!(
            minimum_days_in_first_week(gregorian()).unwrap(),
            SwiftValue::int(1)
        );
    }

    #[test]
    fn symbol_access_rejects_non_calendar_receiver() {
        assert!(months_long(SwiftValue::int(0)).is_err());
    }

    /// A `StdContext` whose clock is pinned to a fixed unix instant.
    struct FixedClock(f64, Vec<u8>);
    impl StdContext for FixedClock {
        fn call_closure(&mut self, _id: usize, _args: Vec<SwiftValue>) -> StdResult {
            unreachable!("calendar predicates never call closures")
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            &mut self.1
        }
        fn now_unix_seconds(&mut self) -> f64 {
            self.0
        }
    }

    fn date_at(y: i64, m: i64, d: i64) -> SwiftValue {
        date_value(ref_seconds_from_ymdhms(y, m, d, 12, 0, 0))
    }

    fn unwrap_bool(outcome: Outcome) -> bool {
        match outcome.result {
            SwiftValue::Bool(b) => b,
            other => panic!("expected Bool, got {other:?}"),
        }
    }

    #[test]
    fn day_of_year_counts_from_january_first() {
        // 2024-06-29 in a leap year: 31+29+31+30+31+29 = 181.
        let civil = decompose(ref_seconds_from_ymdhms(2024, 6, 29, 0, 0, 0));
        assert_eq!(component_value(&civil, "dayOfYear"), Some(181));
        let jan1 = decompose(ref_seconds_from_ymdhms(2024, 1, 1, 0, 0, 0));
        assert_eq!(component_value(&jan1, "dayOfYear"), Some(1));
    }

    #[test]
    fn weekend_detection_uses_saturday_and_sunday() {
        let mut ctx = FixedClock(0.0, Vec::new());
        // 2024-06-29 Sat, 2024-06-30 Sun, 2024-07-01 Mon.
        assert!(unwrap_bool(
            calendar_is_date_in_weekend(&mut ctx, gregorian(), vec![date_at(2024, 6, 29)]).unwrap()
        ));
        assert!(unwrap_bool(
            calendar_is_date_in_weekend(&mut ctx, gregorian(), vec![date_at(2024, 6, 30)]).unwrap()
        ));
        assert!(!unwrap_bool(
            calendar_is_date_in_weekend(&mut ctx, gregorian(), vec![date_at(2024, 7, 1)]).unwrap()
        ));
    }

    #[test]
    fn today_yesterday_tomorrow_relative_to_clock() {
        // Pin "now" to 2024-06-15 18:00 UTC.
        let now = ref_seconds_from_ymdhms(2024, 6, 15, 18, 0, 0) + REFERENCE_DATE_UNIX_OFFSET;
        let mut ctx = FixedClock(now, Vec::new());
        assert!(unwrap_bool(
            calendar_is_date_in_today(&mut ctx, gregorian(), vec![date_at(2024, 6, 15)]).unwrap()
        ));
        assert!(unwrap_bool(
            calendar_is_date_in_yesterday(&mut ctx, gregorian(), vec![date_at(2024, 6, 14)])
                .unwrap()
        ));
        assert!(unwrap_bool(
            calendar_is_date_in_tomorrow(&mut ctx, gregorian(), vec![date_at(2024, 6, 16)])
                .unwrap()
        ));
        assert!(!unwrap_bool(
            calendar_is_date_in_today(&mut ctx, gregorian(), vec![date_at(2024, 6, 16)]).unwrap()
        ));
    }
}
