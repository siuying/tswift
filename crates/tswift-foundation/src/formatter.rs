//! `DateFormatter` and `ISO8601DateFormatter` — Date ⇄ String conversion.
//!
//! Both formatters operate in the proleptic Gregorian calendar in **UTC** and
//! ignore locale (en_US-style names are hard-coded). This diverges from Darwin,
//! which honours `Locale` and `TimeZone`; the gap is documented in
//! `frameworks/foundation/scope.toml`. Formatting/parsing is pure Rust against
//! the civil-date helpers in [`crate::calendar`].

use std::{cell::RefCell, rc::Rc};

use tswift_core::{
    Arg, BuiltinReceiver, ClassObj, Interpreter, IntrinsicFn, MethodEntry, Outcome, StdContext,
    StdError, StdResult, SwiftValue,
};

use crate::{
    calendar::{decompose, ref_seconds_from_ymdhms, Civil},
    date_seconds, date_value, type_error,
};

const MONTH_NAMES: &[&str] = &[
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

const QUARTER_NAMES: &[&str] = &["1st quarter", "2nd quarter", "3rd quarter", "4th quarter"];

const WEEKDAY_NAMES: &[&str] = &[
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

pub fn install(interp: &mut Interpreter<'_>) {
    interp.register_builtin_enum_with_raw(
        "DateFormatter.Style",
        &[
            ("none", 0),
            ("short", 1),
            ("medium", 2),
            ("long", 3),
            ("full", 4),
        ],
    );

    interp.register_free_fn("DateFormatter", date_formatter_init);
    interp.register_property(
        BuiltinReceiver::DateFormatter,
        "dateFormat",
        date_formatter_date_format,
    );
    interp.register_property(
        BuiltinReceiver::DateFormatter,
        "dateStyle",
        date_formatter_date_style,
    );
    interp.register_property(
        BuiltinReceiver::DateFormatter,
        "timeStyle",
        date_formatter_time_style,
    );
    for (name, func) in [
        ("string", date_formatter_string as IntrinsicFn),
        ("date", date_formatter_date),
    ] {
        interp.register_intrinsic(
            BuiltinReceiver::DateFormatter,
            name,
            MethodEntry {
                mutating: false,
                func,
            },
        );
    }

    interp.register_free_fn("ISO8601DateFormatter", iso8601_init);
    interp.register_property(
        BuiltinReceiver::ISO8601DateFormatter,
        "formatOptions",
        iso8601_format_options,
    );
    for (name, func) in [
        ("string", iso8601_string as IntrinsicFn),
        ("date", iso8601_date),
    ] {
        interp.register_intrinsic(
            BuiltinReceiver::ISO8601DateFormatter,
            name,
            MethodEntry {
                mutating: false,
                func,
            },
        );
    }
}

// ---------------------------------------------------------------------------
// Pattern formatting (a focused subset of Unicode date field symbols)
// ---------------------------------------------------------------------------

/// Format a civil instant against a `DateFormatter` pattern. Supported field
/// symbols: `y`, `M` (1–2 numeric, 3 short name, 4 full name), `d`, `H`, `h`,
/// `m`, `s`, `a`, `E` (4 = full weekday name). `'literal'` quoting is honoured;
/// other characters pass through. Unsupported symbols pass through verbatim.
pub(crate) fn format_pattern(civil: &Civil, pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            // Quoted literal; '' is an escaped single quote.
            i += 1;
            while i < chars.len() {
                if chars[i] == '\'' {
                    if i + 1 < chars.len() && chars[i + 1] == '\'' {
                        out.push('\'');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                out.push(chars[i]);
                i += 1;
            }
            continue;
        }
        if c.is_ascii_alphabetic() {
            let mut count = 1;
            while i + count < chars.len() && chars[i + count] == c {
                count += 1;
            }
            out.push_str(&format_field(civil, c, count));
            i += count;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn hour12(hour: i64) -> i64 {
    let h = hour % 12;
    if h == 0 {
        12
    } else {
        h
    }
}

fn format_field(civil: &Civil, symbol: char, count: usize) -> String {
    match symbol {
        'y' | 'Y' => {
            if count == 2 {
                format!("{:02}", civil.year.rem_euclid(100))
            } else {
                format!("{:0width$}", civil.year, width = count.max(1))
            }
        }
        'M' | 'L' => match count {
            1 => format!("{}", civil.month),
            2 => format!("{:02}", civil.month),
            3 => MONTH_NAMES[(civil.month - 1) as usize][..3].to_string(),
            // CLDR: MMMMM == narrow (single letter); MMMM == wide (full name).
            5 => MONTH_NAMES[(civil.month - 1) as usize][..1].to_string(),
            _ => MONTH_NAMES[(civil.month - 1) as usize].to_string(),
        },
        'd' => {
            if count >= 2 {
                format!("{:02}", civil.day)
            } else {
                format!("{}", civil.day)
            }
        }
        'H' => pad(civil.hour, count),
        'h' => pad(hour12(civil.hour), count),
        'm' => pad(civil.minute, count),
        's' => pad(civil.second, count),
        'a' => if civil.hour < 12 { "AM" } else { "PM" }.to_string(),
        'E' => {
            let name = WEEKDAY_NAMES[(civil.weekday - 1) as usize];
            match count {
                // CLDR: EEEEEE == short ("Fr"), EEEEE == narrow ("F"),
                // EEEE == wide ("Friday"), else abbreviated ("Fri").
                6 => name[..2].to_string(),
                5 => name[..1].to_string(),
                4 => name.to_string(),
                _ => name[..3].to_string(),
            }
        }
        'Q' | 'q' => {
            let q = crate::calendar::quarter_of(civil);
            match count {
                1 => format!("{q}"),
                2 => format!("{q:02}"),
                3 => format!("Q{q}"),
                _ => QUARTER_NAMES[(q - 1) as usize].to_string(),
            }
        }
        'G' => {
            // Proleptic-Gregorian era from the astronomical year sign.
            let ad = civil.year > 0;
            match count {
                5 => if ad { "A" } else { "B" }.to_string(),
                4 => if ad { "Anno Domini" } else { "Before Christ" }.to_string(),
                _ => if ad { "AD" } else { "BC" }.to_string(),
            }
        }
        'D' => pad(crate::calendar::day_of_year(civil), count),
        // Unsupported symbol: emit it verbatim so the gap is visible.
        other => other.to_string().repeat(count),
    }
}

fn pad(value: i64, count: usize) -> String {
    if count >= 2 {
        format!("{value:02}")
    } else {
        format!("{value}")
    }
}

/// Parse a string against a numeric-only pattern (`yyyy`, `MM`, `dd`, `HH`,
/// `mm`, `ss`, and literals). Returns `None` if the input does not match.
fn parse_pattern(input: &str, pattern: &str) -> Option<f64> {
    let pchars: Vec<char> = pattern.chars().collect();
    let ichars: Vec<char> = input.chars().collect();
    let (mut pi, mut ii) = (0, 0);
    let (mut year, mut month, mut day) = (1_i64, 1_i64, 1_i64);
    let (mut hour, mut minute, mut second) = (0_i64, 0_i64, 0_i64);
    while pi < pchars.len() {
        let c = pchars[pi];
        if c == '\'' {
            // Quoted literal in the pattern: match its contents verbatim.
            pi += 1;
            while pi < pchars.len() {
                if pchars[pi] == '\'' {
                    if pi + 1 < pchars.len() && pchars[pi + 1] == '\'' {
                        if ii >= ichars.len() || ichars[ii] != '\'' {
                            return None;
                        }
                        ii += 1;
                        pi += 2;
                        continue;
                    }
                    pi += 1;
                    break;
                }
                if ii >= ichars.len() || ichars[ii] != pchars[pi] {
                    return None;
                }
                ii += 1;
                pi += 1;
            }
            continue;
        }
        if c.is_ascii_alphabetic() {
            let mut count = 1;
            while pi + count < pchars.len() && pchars[pi + count] == c {
                count += 1;
            }
            let (value, consumed) = read_int(&ichars, ii, count)?;
            match c {
                'y' | 'Y' => year = value,
                'M' => month = value,
                'd' => day = value,
                'H' | 'h' => hour = value,
                'm' => minute = value,
                's' => second = value,
                _ => return None,
            }
            ii += consumed;
            pi += count;
        } else {
            // Literal must match exactly.
            if ii >= ichars.len() || ichars[ii] != c {
                return None;
            }
            ii += 1;
            pi += 1;
        }
    }
    if ii != ichars.len() {
        return None;
    }
    // Reject out-of-range fields rather than silently normalising (Feb 30 → Mar).
    if !(1..=12).contains(&month)
        || day < 1
        || day > crate::calendar::days_in_month(year, month)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=60).contains(&second)
    {
        return None;
    }
    Some(ref_seconds_from_ymdhms(
        year, month, day, hour, minute, second,
    ))
}

/// Read up to `max` digits starting at `start`, requiring at least one.
fn read_int(chars: &[char], start: usize, max: usize) -> Option<(i64, usize)> {
    let mut value = 0_i64;
    let mut n = 0;
    while start + n < chars.len() && n < max && chars[start + n].is_ascii_digit() {
        value = value * 10 + (chars[start + n] as i64 - '0' as i64);
        n += 1;
    }
    if n == 0 {
        None
    } else {
        Some((value, n))
    }
}

// ---------------------------------------------------------------------------
// DateFormatter
// ---------------------------------------------------------------------------

/// Construct a `DateFormatter` Object (reference semantics — class in real Swift).
fn date_formatter_object(
    date_format: SwiftValue,
    date_style: i128,
    time_style: i128,
) -> SwiftValue {
    SwiftValue::Object(Rc::new(RefCell::new(ClassObj {
        class_name: "DateFormatter".into(),
        fields: vec![
            ("dateFormat".into(), date_format),
            ("dateStyle".into(), SwiftValue::int(date_style)),
            ("timeStyle".into(), SwiftValue::int(time_style)),
        ],
    })))
}

fn date_formatter_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if !args.is_empty() {
        return Err(type_error("DateFormatter() takes no arguments"));
    }
    Ok(date_formatter_object(SwiftValue::Nil, 0, 0))
}

/// Read a named field from either a `SwiftValue::Struct` or `SwiftValue::Object` receiver.
///
/// Returns `None` when the receiver is the wrong type or the field is absent.
fn read_formatter_field(recv: &SwiftValue, ty: &str, field: &str) -> Option<SwiftValue> {
    match recv {
        SwiftValue::Struct(obj) if obj.type_name == ty => obj.get(field).cloned(),
        SwiftValue::Object(o) if o.borrow().class_name == ty => o.borrow().get(field).cloned(),
        _ => None,
    }
}

/// Return `Err` if `recv` is not a `ty` Struct or Object receiver.
fn check_formatter_recv(recv: &SwiftValue, ty: &str) -> Result<(), StdError> {
    match recv {
        SwiftValue::Struct(obj) if obj.type_name == ty => Ok(()),
        SwiftValue::Object(o) if o.borrow().class_name == ty => Ok(()),
        other => Err(type_error(format!(
            "expected {ty}, got {}",
            other.type_name()
        ))),
    }
}

fn date_formatter_date_format(recv: SwiftValue) -> StdResult {
    check_formatter_recv(&recv, "DateFormatter")?;
    Ok(read_formatter_field(&recv, "DateFormatter", "dateFormat").unwrap_or(SwiftValue::Nil))
}

fn date_formatter_date_style(recv: SwiftValue) -> StdResult {
    check_formatter_recv(&recv, "DateFormatter")?;
    Ok(read_formatter_field(&recv, "DateFormatter", "dateStyle").unwrap_or(SwiftValue::int(0)))
}

fn date_formatter_time_style(recv: SwiftValue) -> StdResult {
    check_formatter_recv(&recv, "DateFormatter")?;
    Ok(read_formatter_field(&recv, "DateFormatter", "timeStyle").unwrap_or(SwiftValue::int(0)))
}

/// Resolve a style field that may be stored as an `Int` or a `.style` enum.
fn style_ordinal(value: Option<&SwiftValue>) -> i64 {
    match value {
        Some(SwiftValue::Int(i)) => i.raw as i64,
        Some(SwiftValue::Enum(obj)) => match obj.case.as_str() {
            "none" => 0,
            "short" => 1,
            "medium" => 2,
            "long" => 3,
            "full" => 4,
            _ => 0,
        },
        _ => 0,
    }
}

fn date_style_pattern(style: i64) -> &'static str {
    match style {
        1 => "M/d/yy",
        2 => "MMM d, yyyy",
        3 => "MMMM d, yyyy",
        4 => "EEEE, MMMM d, yyyy",
        _ => "",
    }
}

fn time_style_pattern(style: i64) -> &'static str {
    match style {
        1 => "h:mm a",
        2..=4 => "h:mm:ss a",
        _ => "",
    }
}

/// The effective pattern: an explicit `dateFormat`, else date/time styles.
///
/// Accepts both `SwiftValue::Struct` and `SwiftValue::Object` receivers.
fn effective_pattern(recv: &SwiftValue) -> String {
    if let Some(SwiftValue::Str(fmt)) = read_formatter_field(recv, "DateFormatter", "dateFormat") {
        return fmt.to_string();
    }
    let date = date_style_pattern(style_ordinal(
        read_formatter_field(recv, "DateFormatter", "dateStyle").as_ref(),
    ));
    let time = time_style_pattern(style_ordinal(
        read_formatter_field(recv, "DateFormatter", "timeStyle").as_ref(),
    ));
    match (date.is_empty(), time.is_empty()) {
        (false, false) => format!("{date} {time}"),
        (false, true) => date.to_string(),
        (true, false) => time.to_string(),
        (true, true) => String::new(),
    }
}

fn date_formatter_string(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let [date] = args.as_slice() else {
        return Err(type_error(
            "DateFormatter.string(from:) expects one argument",
        ));
    };
    check_formatter_recv(&recv, "DateFormatter")?;
    let pattern = effective_pattern(&recv);
    let civil = decompose(date_seconds(date)?);
    Ok(Outcome {
        result: SwiftValue::Str(format_pattern(&civil, &pattern)),
        receiver: recv,
    })
}

fn date_formatter_date(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let [SwiftValue::Str(input)] = args.as_slice() else {
        return Err(type_error(
            "DateFormatter.date(from:) expects a String argument",
        ));
    };
    check_formatter_recv(&recv, "DateFormatter")?;
    let pattern = effective_pattern(&recv);
    let result = match parse_pattern(input, &pattern) {
        Some(seconds) => date_value(seconds),
        None => SwiftValue::Nil,
    };
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

// ---------------------------------------------------------------------------
// ISO8601DateFormatter
// ---------------------------------------------------------------------------

const ISO8601_PATTERN: &str = "yyyy-MM-dd'T'HH:mm:ss";

/// Construct an `ISO8601DateFormatter` Object (reference semantics — class in real Swift).
fn iso8601_object(options: i128) -> SwiftValue {
    SwiftValue::Object(Rc::new(RefCell::new(ClassObj {
        class_name: "ISO8601DateFormatter".into(),
        fields: vec![("formatOptions".into(), SwiftValue::int(options))],
    })))
}

fn iso8601_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if !args.is_empty() {
        return Err(type_error("ISO8601DateFormatter() takes no arguments"));
    }
    // 1 == `.withInternetDateTime`, the Darwin default.
    Ok(iso8601_object(1))
}

fn iso8601_format_options(recv: SwiftValue) -> StdResult {
    check_formatter_recv(&recv, "ISO8601DateFormatter")?;
    Ok(
        read_formatter_field(&recv, "ISO8601DateFormatter", "formatOptions")
            .unwrap_or(SwiftValue::int(1)),
    )
}

fn iso8601_string(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let [date] = args.as_slice() else {
        return Err(type_error(
            "ISO8601DateFormatter.string(from:) expects one argument",
        ));
    };
    let civil = decompose(date_seconds(date)?);
    let body = format_pattern(&civil, ISO8601_PATTERN);
    Ok(Outcome {
        result: SwiftValue::Str(format!("{body}Z")),
        receiver: recv,
    })
}

fn iso8601_date(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let [SwiftValue::Str(input)] = args.as_slice() else {
        return Err(type_error(
            "ISO8601DateFormatter.date(from:) expects a String argument",
        ));
    };
    // Tolerate the trailing UTC designator (`Z`).
    let trimmed = input.strip_suffix('Z').unwrap_or(input);
    let result = match parse_pattern(trimmed, ISO8601_PATTERN) {
        Some(seconds) => date_value(seconds),
        None => SwiftValue::Nil,
    };
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{date_value, REFERENCE_DATE_UNIX_OFFSET};

    /// Minimal `StdContext` for formatter unit tests: formatters never invoke
    /// closures or write output, so both required methods are unreachable.
    struct PanicCtx;
    impl StdContext for PanicCtx {
        fn call_closure(&mut self, _id: usize, _args: Vec<SwiftValue>) -> StdResult {
            unreachable!("formatter helpers never call closures")
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            unreachable!("formatter helpers never write output")
        }
    }

    fn civil_at(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> Civil {
        decompose(ref_seconds_from_ymdhms(y, mo, d, h, mi, s))
    }

    #[test]
    fn formats_numeric_pattern() {
        let civil = civil_at(2024, 6, 29, 9, 4, 5);
        assert_eq!(
            format_pattern(&civil, "yyyy-MM-dd HH:mm:ss"),
            "2024-06-29 09:04:05"
        );
    }

    #[test]
    fn formats_named_fields() {
        let civil = civil_at(2024, 6, 29, 14, 30, 0);
        assert_eq!(
            format_pattern(&civil, "EEEE, MMMM d, yyyy"),
            "Saturday, June 29, 2024"
        );
        assert_eq!(format_pattern(&civil, "h:mm a"), "2:30 PM");
    }

    #[test]
    fn round_trips_numeric_parse() {
        let seconds = ref_seconds_from_ymdhms(2024, 6, 29, 9, 4, 5);
        let parsed = parse_pattern("2024-06-29 09:04:05", "yyyy-MM-dd HH:mm:ss").unwrap();
        assert_eq!(parsed, seconds);
    }

    // ----- Phase 2 reference-semantics tests --------------------------------

    #[test]
    fn date_formatter_init_returns_object() {
        let result = date_formatter_init(&mut PanicCtx, vec![]).unwrap();
        assert!(
            matches!(&result, SwiftValue::Object(o) if o.borrow().class_name == "DateFormatter"),
            "expected Object with class_name DateFormatter, got {result:?}"
        );
    }

    #[test]
    fn iso8601_init_returns_object() {
        let result = iso8601_init(&mut PanicCtx, vec![]).unwrap();
        assert!(
            matches!(&result, SwiftValue::Object(o)
                if o.borrow().class_name == "ISO8601DateFormatter"),
            "expected Object with class_name ISO8601DateFormatter, got {result:?}"
        );
    }

    /// `let f = DateFormatter()` — an alias of `f` observes a property mutation
    /// written through the alias (reference semantics, Swift class behaviour).
    #[test]
    fn date_formatter_alias_observes_property_change() {
        let f = date_formatter_object(SwiftValue::Nil, 0, 0);
        let alias = f.clone(); // shallow Rc clone — same ClassObj
        if let SwiftValue::Object(o) = &alias {
            o.borrow_mut()
                .set("dateFormat", SwiftValue::Str("yyyy".into()));
        } else {
            panic!("alias was not Object");
        }
        // The original binding must see the mutation.
        let field = read_formatter_field(&f, "DateFormatter", "dateFormat");
        assert_eq!(
            field,
            Some(SwiftValue::Str("yyyy".into())),
            "original did not observe alias mutation"
        );
    }

    /// `string(from:)` uses the `dateFormat` stored in the Object, so a
    /// property set before the call is reflected in the output.
    #[test]
    fn date_formatter_string_reflects_object_date_format() {
        let obj = date_formatter_object(SwiftValue::Str("yyyy-MM-dd".into()), 0, 0);
        let date = date_value(ref_seconds_from_ymdhms(2024, 6, 29, 0, 0, 0));
        let out = date_formatter_string(&mut PanicCtx, obj, vec![date]).unwrap();
        assert_eq!(out.result, SwiftValue::Str("2024-06-29".into()));
    }

    // ----- existing tests (unchanged) ---------------------------------------

    #[test]
    fn iso8601_emits_internet_date_time() {
        // Reference date (timeIntervalSinceReferenceDate 0) is 2001-01-01T00:00:00Z.
        let civil = decompose(0.0);
        let body = format_pattern(&civil, ISO8601_PATTERN);
        assert_eq!(format!("{body}Z"), "2001-01-01T00:00:00Z");
        let _ = REFERENCE_DATE_UNIX_OFFSET;
    }
}
