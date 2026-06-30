//! `Calendar` — Gregorian/UTC date arithmetic.
//!
//! The runtime models a single calendar: the proleptic Gregorian calendar in
//! UTC. Date ⇄ component conversion uses Howard Hinnant's `days_from_civil` /
//! `civil_from_days` algorithms (no external crates, offline build). This
//! diverges from Darwin, which honours locale and time zone; that gap is
//! intentional for now.

use std::rc::Rc;

use tswift_core::{
    Arg, BuiltinReceiver, Interpreter, IntrinsicFn, LabeledMethodEntry, MethodEntry, Outcome,
    StdContext, StdError, StdResult, StructObj, SwiftValue,
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
];

pub fn install(interp: &mut Interpreter<'_>) {
    interp.register_builtin_enum("Calendar.Component", CALENDAR_COMPONENTS);
    interp.register_builtin_enum("Calendar.Identifier", &["gregorian"]);

    interp.register_free_fn("Calendar", calendar_init);
    interp.register_static(BuiltinReceiver::Calendar, "current", calendar_current);
    interp.register_property(BuiltinReceiver::Calendar, "identifier", calendar_identifier);

    for (name, mutating, func) in [
        (
            "dateComponents",
            false,
            calendar_date_components as IntrinsicFn,
        ),
        ("component", false, calendar_component),
        ("startOfDay", false, calendar_start_of_day),
        ("isDate", false, calendar_is_date_in_same_day),
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
    // 1970-01-01 (day 0) is a Thursday; Swift weekday is 1=Sunday..7=Saturday.
    let weekday = ((days % 7 + 7) % 7 + 4) % 7 + 1;
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

fn calendar_value(identifier: &str) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "Calendar".into(),
        fields: vec![("_identifier".into(), SwiftValue::Str(identifier.into()))],
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
}
