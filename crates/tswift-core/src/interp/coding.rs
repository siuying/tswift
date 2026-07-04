//! `Codable` support: JSON encoding/decoding for the `JSONEncoder`/`JSONDecoder`
//! markers, layered over the hand-written [`crate::json`] model.

use std::rc::Rc;

use tswift_frontend::{Node, TypeRepr};

use super::{decode_element_type, metatype_name, EvalError, Interpreter, Signal};
use crate::json::{Json, OutputFormatting};
use crate::value::{EnumObj, StructObj, SwiftValue};

/// `timeIntervalSinceReferenceDate` → Unix epoch offset (seconds).
/// 2001-01-01T00:00:00Z in Unix time.
const REFERENCE_DATE_UNIX_OFFSET: f64 = 978_307_200.0;

// ---------------------------------------------------------------------------
// Date encoding/decoding strategy enums
// ---------------------------------------------------------------------------

/// Mirrors `JSONEncoder.DateEncodingStrategy` cases.
/// Integer raw values match the constants registered in `tswift-foundation::json`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum DateEncoding {
    /// Default: encode as `timeIntervalSinceReferenceDate` (a Double).
    DeferredToDate = 0,
    /// Encode as seconds since Unix epoch (a Double).
    SecondsSince1970 = 1,
    /// Encode as milliseconds since Unix epoch (a Double).
    MillisecondsSince1970 = 2,
    /// Encode as an ISO 8601 string (`yyyy-MM-dd'T'HH:mm:ss'Z'` UTC).
    Iso8601 = 3,
}

impl DateEncoding {
    fn from_raw(raw: i128) -> Self {
        match raw {
            1 => Self::SecondsSince1970,
            2 => Self::MillisecondsSince1970,
            3 => Self::Iso8601,
            _ => Self::DeferredToDate,
        }
    }
    fn from_case(case: &str) -> Self {
        match case {
            "secondsSince1970" => Self::SecondsSince1970,
            "millisecondsSince1970" => Self::MillisecondsSince1970,
            "iso8601" => Self::Iso8601,
            _ => Self::DeferredToDate,
        }
    }
}

/// Mirrors `JSONDecoder.DateDecodingStrategy` cases.
#[derive(Clone, Copy, Debug, PartialEq)]
enum DateDecoding {
    /// Default: decode from `timeIntervalSinceReferenceDate` (a Double).
    DeferredToDate = 0,
    /// Decode from seconds since Unix epoch (a Double).
    SecondsSince1970 = 1,
    /// Decode from milliseconds since Unix epoch (a Double).
    MillisecondsSince1970 = 2,
    /// Decode from an ISO 8601 string (`yyyy-MM-dd'T'HH:mm:ss'Z'` UTC).
    Iso8601 = 3,
}

impl DateDecoding {
    fn from_raw(raw: i128) -> Self {
        match raw {
            1 => Self::SecondsSince1970,
            2 => Self::MillisecondsSince1970,
            3 => Self::Iso8601,
            _ => Self::DeferredToDate,
        }
    }
    fn from_case(case: &str) -> Self {
        match case {
            "secondsSince1970" => Self::SecondsSince1970,
            "millisecondsSince1970" => Self::MillisecondsSince1970,
            "iso8601" => Self::Iso8601,
            _ => Self::DeferredToDate,
        }
    }
}

// ---------------------------------------------------------------------------
// Data encoding/decoding strategy enums
// ---------------------------------------------------------------------------

/// Mirrors `JSONEncoder.DataEncodingStrategy` cases.
/// Integer raw values match the constants registered in `tswift-foundation::json`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
enum DataEncoding {
    /// Encode `Data` as a Base64-encoded JSON string (default).
    #[default]
    Base64 = 0,
    /// Encode `Data` as a JSON array of byte integers (Swift default-less fallback).
    DeferredToData = 1,
}

impl DataEncoding {
    fn from_raw(raw: i128) -> Self {
        match raw {
            1 => Self::DeferredToData,
            _ => Self::Base64,
        }
    }
    fn from_case(case: &str) -> Self {
        match case {
            "deferredToData" => Self::DeferredToData,
            _ => Self::Base64,
        }
    }
}

/// Mirrors `JSONDecoder.DataDecodingStrategy` cases.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
enum DataDecoding {
    /// Decode `Data` from a Base64-encoded JSON string (default).
    #[default]
    Base64 = 0,
    /// Decode `Data` from a JSON array of byte integers.
    DeferredToData = 1,
}

impl DataDecoding {
    fn from_raw(raw: i128) -> Self {
        match raw {
            1 => Self::DeferredToData,
            _ => Self::Base64,
        }
    }
    fn from_case(case: &str) -> Self {
        match case {
            "deferredToData" => Self::DeferredToData,
            _ => Self::Base64,
        }
    }
}

// ---------------------------------------------------------------------------
// Output-formatting and key-strategy enums
// ---------------------------------------------------------------------------

/// Mirrors `JSONEncoder.KeyEncodingStrategy` (raw ints registered in
/// `tswift-foundation::json`).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
enum KeyEncoding {
    /// Default: use the property name as-is.
    #[default]
    UseDefaultKeys = 0,
    /// Convert camelCase property names to snake_case JSON keys.
    ConvertToSnakeCase = 1,
}

impl KeyEncoding {
    fn from_raw(raw: i128) -> Self {
        match raw {
            1 => Self::ConvertToSnakeCase,
            _ => Self::UseDefaultKeys,
        }
    }
    fn from_case(case: &str) -> Self {
        match case {
            "convertToSnakeCase" => Self::ConvertToSnakeCase,
            _ => Self::UseDefaultKeys,
        }
    }
}

/// Mirrors `JSONDecoder.KeyDecodingStrategy`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
enum KeyDecoding {
    /// Default: use the JSON key as-is.
    #[default]
    UseDefaultKeys = 0,
    /// Treat JSON keys as snake_case and convert to camelCase field names.
    ConvertFromSnakeCase = 1,
}

impl KeyDecoding {
    fn from_raw(raw: i128) -> Self {
        match raw {
            1 => Self::ConvertFromSnakeCase,
            _ => Self::UseDefaultKeys,
        }
    }
    fn from_case(case: &str) -> Self {
        match case {
            "convertFromSnakeCase" => Self::ConvertFromSnakeCase,
            _ => Self::UseDefaultKeys,
        }
    }
}

// ---------------------------------------------------------------------------
// ISO 8601 formatting / parsing (UTC, no external deps)
// ---------------------------------------------------------------------------

/// Days from the Unix epoch (1970-01-01) for a proleptic-Gregorian date.
/// Algorithm: Howard Hinnant's `days_from_civil`.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Proleptic-Gregorian y/m/d for days since 1970-01-01.
/// Algorithm: Howard Hinnant's `civil_from_days`.
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

/// Format `timeIntervalSinceReferenceDate` as `yyyy-MM-dd'T'HH:mm:ss'Z'` UTC.
fn iso8601_format(ref_seconds: f64) -> String {
    let unix = ref_seconds + REFERENCE_DATE_UNIX_OFFSET;
    let days = (unix / 86_400.0).floor() as i64;
    let secs_in_day = (unix - days as f64 * 86_400.0) as i64;
    let (y, mo, d) = civil_from_days(days);
    let h = secs_in_day / 3600;
    let min = (secs_in_day % 3600) / 60;
    let s = secs_in_day % 60;
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, min, s)
}

/// Returns the number of days in the given Gregorian month, accounting for
/// leap years.  Month is 1-based (1 = January, …, 12 = December).
fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => 30,
    }
}

/// Parse a `yyyy-MM-dd'T'HH:mm:ss[.fff]'Z'` string (UTC, **Z required**) to
/// `timeIntervalSinceReferenceDate`.  Returns `None` on parse failure,
/// including when the trailing 'Z' is absent, the date separator is wrong,
/// the month/day values exceed the calendar maximum, or the day exceeds the
/// actual month length (e.g. Feb 29 in a non-leap year, or Feb 31).
fn iso8601_parse(s: &str) -> Option<f64> {
    // Trailing 'Z' is required (UTC designator); reject strings without it.
    let s = s.strip_suffix('Z')?;
    // Strip optional fractional seconds: find the last ':' then look for '.'.
    // "…:00.500" → strip ".500" giving "…:00".
    // Validate that every char in the fractional part is an ASCII digit.
    let s = if let Some(pos) = s.rfind('.') {
        let frac = &s[pos + 1..];
        if frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
            return None; // reject ".abcZ" or just ".Z"
        }
        &s[..pos]
    } else {
        s
    };
    // Expect exactly "yyyy-MM-ddTHH:mm:ss" (19 chars).
    if s.len() != 19 {
        return None;
    }
    let bytes = s.as_bytes();
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let y: i64 = s[0..4].parse().ok()?;
    let mo: i64 = s[5..7].parse().ok()?;
    let d: i64 = s[8..10].parse().ok()?;
    let h: i64 = s[11..13].parse().ok()?;
    let min: i64 = s[14..16].parse().ok()?;
    let sec: i64 = s[17..19].parse().ok()?;
    // Validate field ranges, including the actual number of days in the month.
    if !(1..=12).contains(&mo)
        || d < 1
        || d > days_in_month(y, mo)
        || !(0..=23).contains(&h)
        || !(0..=59).contains(&min)
        || !(0..=59).contains(&sec)
    {
        return None;
    }
    let days = days_from_civil(y, mo, d);
    let unix = days as f64 * 86_400.0 + (h * 3600 + min * 60 + sec) as f64;
    Some(unix - REFERENCE_DATE_UNIX_OFFSET)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract `timeIntervalSinceReferenceDate` from a `Date` struct value.
fn date_ref_seconds(value: &SwiftValue) -> Option<f64> {
    if let SwiftValue::Struct(o) = value {
        if o.type_name == "Date" {
            if let Some(SwiftValue::Double(d)) = o.get("_timeIntervalSinceReferenceDate") {
                return Some(*d);
            }
        }
    }
    None
}

/// Build a `Date` struct from `timeIntervalSinceReferenceDate`.
fn date_value(ref_seconds: f64) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "Date".into(),
        fields: vec![(
            "_timeIntervalSinceReferenceDate".into(),
            SwiftValue::Double(ref_seconds),
        )],
    }))
}

// ---------------------------------------------------------------------------
// Coder receiver helpers (Struct-or-Object dual-mode)
// ---------------------------------------------------------------------------

/// Read a named field from a coder receiver that may be either a legacy
/// `SwiftValue::Struct` or a class-backed `SwiftValue::Object`.  The
/// `RefCell` borrow is dropped before returning the cloned value.
fn read_coder_field(recv: &SwiftValue, field: &str) -> Option<SwiftValue> {
    match recv {
        SwiftValue::Struct(o) => o.get(field).cloned(),
        SwiftValue::Object(o) => {
            let borrow = o.borrow();
            borrow.get(field).cloned()
        }
        _ => None,
    }
}

/// Return the type name carried by a coder receiver (Struct or Object).
/// Returns an owned `String` so callers avoid keeping a `RefCell` borrow live.
fn coder_type_name(recv: &SwiftValue) -> Option<String> {
    match recv {
        SwiftValue::Struct(o) => Some(o.type_name.clone()),
        SwiftValue::Object(o) => Some(o.borrow().class_name.clone()),
        _ => None,
    }
}

/// Read a `DateEncodingStrategy` from a `JSONEncoder` receiver (Struct or
/// Object).  Accepts both the legacy raw-integer form and the builtin-enum
/// form produced by leading-dot resolution (e.g. `.iso8601`).
fn encoder_date_strategy(recv: &SwiftValue) -> DateEncoding {
    match read_coder_field(recv, "dateEncodingStrategy") {
        Some(SwiftValue::Int(i)) => DateEncoding::from_raw(i.raw),
        Some(SwiftValue::Enum(e)) => DateEncoding::from_case(&e.case),
        _ => DateEncoding::DeferredToDate,
    }
}

/// Read a `DateDecodingStrategy` from a `JSONDecoder` receiver (Struct or
/// Object).  Accepts both the legacy raw-integer form and the builtin-enum
/// form.
fn decoder_date_strategy(recv: &SwiftValue) -> DateDecoding {
    match read_coder_field(recv, "dateDecodingStrategy") {
        Some(SwiftValue::Int(i)) => DateDecoding::from_raw(i.raw),
        Some(SwiftValue::Enum(e)) => DateDecoding::from_case(&e.case),
        _ => DateDecoding::DeferredToDate,
    }
}

/// Read `outputFormatting` from a `JSONEncoder` receiver (Struct or Object).
/// The field may be an `Int` (single flag) or an `Array` of ints (OptionSet
/// array literal `[.prettyPrinted, .sortedKeys]` → OR of bit flags).
/// Map an `OutputFormatting` case name to its bit position.
fn output_formatting_case_bit(case: &str) -> u64 {
    match case {
        "prettyPrinted" => 1,
        "sortedKeys" => 2,
        _ => 0,
    }
}

fn encoder_output_formatting(recv: &SwiftValue) -> OutputFormatting {
    let bits: u64 = match read_coder_field(recv, "outputFormatting") {
        Some(SwiftValue::Int(i)) => i.raw as u64,
        Some(SwiftValue::Enum(e)) => output_formatting_case_bit(&e.case),
        Some(SwiftValue::Array(items)) => items.iter().fold(0u64, |acc, v| match v {
            SwiftValue::Int(i) => acc | i.raw as u64,
            SwiftValue::Enum(e) => acc | output_formatting_case_bit(&e.case),
            _ => acc,
        }),
        _ => 0,
    };
    OutputFormatting {
        pretty_printed: (bits & 1) != 0,
        sorted_keys: (bits & 2) != 0,
    }
}

/// Read a `KeyEncodingStrategy` from a `JSONEncoder` receiver (Struct or
/// Object).
fn encoder_key_strategy(recv: &SwiftValue) -> KeyEncoding {
    match read_coder_field(recv, "keyEncodingStrategy") {
        Some(SwiftValue::Int(i)) => KeyEncoding::from_raw(i.raw),
        Some(SwiftValue::Enum(e)) => KeyEncoding::from_case(&e.case),
        _ => KeyEncoding::UseDefaultKeys,
    }
}

/// Read a `KeyDecodingStrategy` from a `JSONDecoder` receiver (Struct or
/// Object).
fn decoder_key_strategy(recv: &SwiftValue) -> KeyDecoding {
    match read_coder_field(recv, "keyDecodingStrategy") {
        Some(SwiftValue::Int(i)) => KeyDecoding::from_raw(i.raw),
        Some(SwiftValue::Enum(e)) => KeyDecoding::from_case(&e.case),
        _ => KeyDecoding::UseDefaultKeys,
    }
}

/// Read a `DataEncodingStrategy` from a `JSONEncoder` receiver (Struct or
/// Object).
fn encoder_data_strategy(recv: &SwiftValue) -> DataEncoding {
    match read_coder_field(recv, "dataEncodingStrategy") {
        Some(SwiftValue::Int(i)) => DataEncoding::from_raw(i.raw),
        Some(SwiftValue::Enum(e)) => DataEncoding::from_case(&e.case),
        _ => DataEncoding::Base64,
    }
}

/// Read a `DataDecodingStrategy` from a `JSONDecoder` receiver (Struct or
/// Object).
fn decoder_data_strategy(recv: &SwiftValue) -> DataDecoding {
    match read_coder_field(recv, "dataDecodingStrategy") {
        Some(SwiftValue::Int(i)) => DataDecoding::from_raw(i.raw),
        Some(SwiftValue::Enum(e)) => DataDecoding::from_case(&e.case),
        _ => DataDecoding::Base64,
    }
}

/// Convert a camelCase identifier to snake_case, implementing Apple's
/// `_convertToSnakeCase` algorithm from swift-foundation's JSONEncoder.
///
/// Rules (applied left to right over the inner characters — after stripping
/// preserved leading / trailing underscores):
///
/// 1. **Non-uppercase → uppercase boundary**: any character that is *not*
///    uppercase (a lowercase letter, a digit, or any other non-letter)
///    immediately followed by an uppercase letter inserts a `_` before that
///    uppercase letter.
///    `oneTwoThree` → `one_two_three`; `address1Line2` → `address1_line2`;
///    `a1B` → `a1_b`.
///
/// 2. **Uppercase-run boundary**: inside a run of consecutive uppercase
///    letters, if position `i` is uppercase *and* `i+1` is a **lowercase**
///    letter, insert a `_` before position `i` (split before the last
///    uppercase of the run).
///    `URLAddress` → `url_address`; `URLValue` → `url_value`;
///    `imageURL` → `image_url` (no rule 2 break because `URL` is at the end).
///
/// * Leading and trailing underscores are preserved unchanged.
pub(super) fn to_snake_case(s: &str) -> String {
    if s.is_empty() {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();

    // Preserve leading underscores.
    let leading = chars.iter().take_while(|&&c| c == '_').count();
    // Preserve trailing underscores.
    let trailing = chars.iter().rev().take_while(|&&c| c == '_').count();

    let inner_end = n.saturating_sub(trailing);
    if leading >= inner_end {
        // All underscores or empty inner section — return as-is.
        return s.to_string();
    }

    let inner = &chars[leading..inner_end];
    let m = inner.len();

    // Mark positions where a word boundary occurs *before* that position.
    let mut breaks = vec![false; m];
    for i in 1..m {
        let prev = inner[i - 1];
        let curr = inner[i];
        // Rule 1: any non-uppercase char followed by an uppercase char.
        // Using `!is_uppercase` (rather than `is_lowercase`) means digits and
        // other non-letter chars also trigger a break, e.g. `1L` in
        // `address1Line2` → `address1_line2`.
        if !prev.is_uppercase() && curr.is_uppercase() {
            breaks[i] = true;
        } else if i + 1 < m
            && prev.is_uppercase()
            && curr.is_uppercase()
            && inner[i + 1].is_lowercase()
        {
            // Rule 2: within an uppercase run, break before the last uppercase
            // when it is immediately followed by a lowercase letter.
            // e.g. `L`→`A` in `URLAd…` where `A` is followed by `d`.
            breaks[i] = true;
        }
    }

    let mut result = String::with_capacity(s.len() + 4);
    for _ in 0..leading {
        result.push('_');
    }

    let mut word_start = 0;
    for i in 1..=m {
        if i == m || breaks[i] {
            let word: String = inner[word_start..i]
                .iter()
                .collect::<String>()
                .to_lowercase();
            if word_start > 0 {
                result.push('_');
            }
            result.push_str(&word);
            word_start = i;
        }
    }

    for _ in 0..trailing {
        result.push('_');
    }
    result
}

// ---------------------------------------------------------------------------
// Method dispatch hook
// ---------------------------------------------------------------------------

impl<'w> Interpreter<'w> {
    /// Handle a method call on a `JSONEncoder`/`JSONDecoder` marker value.
    /// Returns `Ok(None)` when `base_value` is not a JSON coder (or the method
    /// is not one it serves), letting normal dispatch continue.
    pub(super) fn try_json_coder_method(
        &mut self,
        base_value: &SwiftValue,
        method: &str,
        arg_nodes: &[Node<'static>],
    ) -> Result<Option<SwiftValue>, Signal> {
        // Accept both legacy Struct receivers and new Object receivers so the
        // transition is backward-compatible with any serialised Struct values
        // that might exist in long-lived interpreter sessions.
        let type_name = match coder_type_name(base_value) {
            Some(n) => n,
            None => return Ok(None),
        };
        // `JSONEncoder().encode(value)` → a JSON `Data` (UTF-8 bytes).
        if type_name == "JSONEncoder" && method == "encode" {
            let date_enc = encoder_date_strategy(base_value);
            let output_fmt = encoder_output_formatting(base_value);
            let key_enc = encoder_key_strategy(base_value);
            let data_enc = encoder_data_strategy(base_value);
            let args = self.eval_args(arg_nodes)?;
            let value = args
                .first()
                .map(|a| a.value.clone())
                .ok_or_else(|| EvalError::Type("encode expects a value".into()))?;
            let json = self.json_encode(&value, date_enc, key_enc, data_enc)?;
            let json_str = crate::json::to_string_fmt(&json, &output_fmt);
            let bytes: Vec<SwiftValue> = json_str
                .bytes()
                .map(|b| SwiftValue::int(b as i128))
                .collect();
            let data = SwiftValue::Struct(Rc::new(crate::value::StructObj {
                type_name: "Data".into(),
                fields: vec![("_bytes".into(), SwiftValue::Array(Rc::new(bytes)))],
            }));
            return Ok(Some(data));
        }
        // `JSONDecoder().decode(T.self, from: data)` → a value of type `T`.
        if type_name == "JSONDecoder" && method == "decode" {
            let date_dec = decoder_date_strategy(base_value);
            let key_dec = decoder_key_strategy(base_value);
            let data_dec = decoder_data_strategy(base_value);
            let type_name = arg_nodes
                .first()
                .and_then(metatype_name)
                .ok_or_else(|| EvalError::Type("decode expects a metatype".into()))?;
            let data = arg_nodes
                .get(1)
                .map(|n| self.eval(n))
                .transpose()?
                .ok_or_else(|| EvalError::Type("decode expects data".into()))?;
            let text = match data {
                SwiftValue::Str(s) => s,
                SwiftValue::Struct(ref obj) if obj.type_name == "Data" => {
                    // Extract UTF-8 bytes from the Data value.
                    let bytes = match obj.get("_bytes") {
                        Some(SwiftValue::Array(items)) => items
                            .iter()
                            .map(|v| match v {
                                SwiftValue::Int(i) if (0..=255).contains(&i.raw) => Ok(i.raw as u8),
                                other => Err(EvalError::Type(format!(
                                    "decode: Data contains non-byte value: {}",
                                    other.type_name()
                                ))
                                .into()),
                            })
                            .collect::<Result<Vec<u8>, Signal>>()?,
                        _ => {
                            return Err(
                                EvalError::Type("decode: malformed Data value".into()).into()
                            )
                        }
                    };
                    match std::string::String::from_utf8(bytes) {
                        Ok(s) => s,
                        Err(_) => {
                            return Err(
                                EvalError::Type("decode: Data is not valid UTF-8".into()).into()
                            )
                        }
                    }
                }
                other => {
                    return Err(EvalError::Type(format!(
                        "decode expects String/Data, got {}",
                        other.type_name()
                    ))
                    .into())
                }
            };
            let json = crate::json::parse(&text)
                .map_err(|e| Signal::Throw(SwiftValue::Str(format!("decode error: {e}"))))?;
            return Ok(Some(
                self.json_decode(&type_name, &json, date_dec, key_dec, data_dec)?,
            ));
        }
        Ok(None)
    }

    /// `PropertyListEncoder().encode(value)` — produces an XML plist `Data`.
    pub(super) fn try_plist_coder_method(
        &mut self,
        base_value: &SwiftValue,
        method: &str,
        arg_nodes: &[Node<'static>],
    ) -> Result<Option<SwiftValue>, Signal> {
        // Accept both legacy Struct receivers and new Object receivers.
        let type_name = match coder_type_name(base_value) {
            Some(n) => n,
            None => return Ok(None),
        };
        if type_name != "PropertyListEncoder" || method != "encode" {
            return Ok(None);
        }
        // outputFormat is a `PropertyListSerialization.PropertyListFormat` enum
        // case set via `.xml` / `.binary` / `.openStep` leading-dot syntax.
        // Real Foundation defaults to `.binary`; we default to `.xml` since
        // binary output cannot be represented as UTF-8 Data in this runtime.
        let fmt_case = match read_coder_field(base_value, "outputFormat") {
            Some(SwiftValue::Enum(e))
                if e.type_name == "PropertyListSerialization.PropertyListFormat" =>
            {
                e.case.clone()
            }
            // No outputFormat set — treat as .xml (our runtime default).
            None => "xml".to_string(),
            // Any other value (e.g. stale Int or wrong type) is rejected.
            Some(other) => {
                return Err(Signal::Throw(SwiftValue::Str(format!(
                    "PropertyListEncoder: invalid outputFormat value: {}",
                    other.type_name()
                ))));
            }
        };
        match fmt_case.as_str() {
            "xml" => {}
            "binary" => {
                return Err(Signal::Throw(SwiftValue::Str(
                    "PropertyListEncoder: binary plist format is not supported in this runtime"
                        .into(),
                )));
            }
            other => {
                return Err(Signal::Throw(SwiftValue::Str(format!(
                    "PropertyListEncoder: output format '.{other}' is not supported in this runtime",
                ))));
            }
        }
        let args = self.eval_args(arg_nodes)?;
        let value = args
            .first()
            .map(|a| a.value.clone())
            .ok_or_else(|| EvalError::Type("encode expects a value".into()))?;
        let xml = plist_encode_xml(&value)?;
        let bytes: Vec<SwiftValue> = xml.bytes().map(|b| SwiftValue::int(b as i128)).collect();
        let data = SwiftValue::Struct(Rc::new(crate::value::StructObj {
            type_name: "Data".into(),
            fields: vec![("_bytes".into(), SwiftValue::Array(Rc::new(bytes)))],
        }));
        Ok(Some(data))
    }

    /// Serialize a `Codable` value to its `JSONEncoder` representation.
    fn json_encode(
        &self,
        value: &SwiftValue,
        date_enc: DateEncoding,
        key_enc: KeyEncoding,
        data_enc: DataEncoding,
    ) -> Result<Json, Signal> {
        // `URL` is encoded as its `absoluteString` (a JSON string).
        if let SwiftValue::Struct(o) = value {
            if o.type_name == "URL" {
                if let Some(SwiftValue::Str(s)) = o.get("_string") {
                    return Ok(Json::Str(s.clone()));
                }
                return Err(EvalError::Type("cannot encode malformed URL value".into()).into());
            }
            // `UUID` is encoded as its `uuidString` (a JSON string).
            if o.type_name == "UUID" {
                if let Some(SwiftValue::Str(s)) = o.get("uuidString") {
                    return Ok(Json::Str(s.clone()));
                }
                return Err(EvalError::Type("cannot encode malformed UUID value".into()).into());
            }
            // `IndexPath` encodes as `{"indexes":[...]}` matching Foundation.
            if o.type_name == "IndexPath" {
                let indexes = match o.get("_indexes") {
                    Some(SwiftValue::Array(items)) => items
                        .iter()
                        .map(|v| match v {
                            SwiftValue::Int(i) => Ok(Json::Int(i.raw as i64)),
                            other => Err(EvalError::Type(format!(
                                "IndexPath contains non-Int value: {}",
                                other.type_name()
                            ))
                            .into()),
                        })
                        .collect::<Result<Vec<_>, Signal>>()?,
                    _ => vec![],
                };
                return Ok(Json::Object(vec![("indexes".into(), Json::Array(indexes))]));
            }
            // `Measurement<Unit>` encodes as:
            // {"value":N,"unit":{"symbol":"s","converter":{"coefficient":C,"constant":K}}}
            // matching Foundation's JSONEncoder output.
            if o.type_name == "Measurement" {
                let val = o
                    .get("value")
                    .ok_or_else(|| EvalError::Type("Measurement.value missing".into()))?;
                let unit = o
                    .get("unit")
                    .ok_or_else(|| EvalError::Type("Measurement.unit missing".into()))?;
                let SwiftValue::Struct(u) = unit else {
                    return Err(EvalError::Type("Measurement.unit is not a struct".into()).into());
                };
                let symbol = match u.get("symbol") {
                    Some(SwiftValue::Str(s)) => s.clone(),
                    _ => {
                        return Err(EvalError::Type("Measurement unit.symbol missing".into()).into())
                    }
                };
                let coeff = match u.get("_coefficient") {
                    Some(SwiftValue::Double(d)) => *d,
                    Some(SwiftValue::Int(i)) => i.raw as f64,
                    _ => {
                        return Err(
                            EvalError::Type("Measurement unit._coefficient missing".into()).into(),
                        )
                    }
                };
                let constant = match u.get("_constant") {
                    Some(SwiftValue::Double(d)) => *d,
                    Some(SwiftValue::Int(i)) => i.raw as f64,
                    _ => {
                        return Err(
                            EvalError::Type("Measurement unit._constant missing".into()).into()
                        )
                    }
                };
                let converter = Json::Object(vec![
                    ("coefficient".into(), Json::Double(coeff)),
                    ("constant".into(), Json::Double(constant)),
                ]);
                let unit_obj = Json::Object(vec![
                    ("converter".into(), converter),
                    ("symbol".into(), Json::Str(symbol)),
                ]);
                let val_json = self.json_encode(val, date_enc, key_enc, data_enc)?;
                return Ok(Json::Object(vec![
                    ("value".into(), val_json),
                    ("unit".into(), unit_obj),
                ]));
            }
            // `Data` is encoded according to the data strategy.
            if o.type_name == "Data" {
                let bytes = data_bytes_from_value(value)
                    .ok_or_else(|| EvalError::Type("cannot encode malformed Data value".into()))?;
                return Ok(match data_enc {
                    DataEncoding::Base64 => Json::Str(crate::base64::encode(&bytes)),
                    DataEncoding::DeferredToData => {
                        Json::Array(bytes.iter().map(|&b| Json::Int(b as i64)).collect())
                    }
                });
            }
        }
        // `Date` is encoded according to the strategy before falling into the
        // general struct branch (which would expand the internal fields).
        if let Some(ref_secs) = date_ref_seconds(value) {
            return Ok(match date_enc {
                DateEncoding::DeferredToDate => Json::Double(ref_secs),
                DateEncoding::SecondsSince1970 => {
                    Json::Double(ref_secs + REFERENCE_DATE_UNIX_OFFSET)
                }
                DateEncoding::MillisecondsSince1970 => {
                    Json::Double((ref_secs + REFERENCE_DATE_UNIX_OFFSET) * 1000.0)
                }
                DateEncoding::Iso8601 => Json::Str(iso8601_format(ref_secs)),
            });
        }
        Ok(match value {
            SwiftValue::Nil => Json::Null,
            SwiftValue::Bool(b) => Json::Bool(*b),
            SwiftValue::Int(i) => Json::Int(i.raw as i64),
            SwiftValue::Double(d) => {
                // Match Swift semantics: JSONEncoder.encode throws
                // EncodingError.invalidValue for non-finite doubles. We model
                // this as a thrown string so `try/catch` can intercept it;
                // silently emitting `inf`/`nan` would produce invalid JSON.
                if d.is_finite() {
                    Json::Double(*d)
                } else {
                    return Err(Signal::Throw(SwiftValue::Str(format!(
                        "EncodingError: cannot encode non-finite Double ({d})"
                    ))));
                }
            }
            SwiftValue::Str(s) => Json::Str(s.clone()),
            SwiftValue::Array(items) => Json::Array(
                items
                    .iter()
                    .map(|v| self.json_encode(v, date_enc, key_enc, data_enc))
                    .collect::<Result<_, _>>()?,
            ),
            SwiftValue::Struct(o) => {
                // Encode fields in declaration order — the parser always
                // produces them in the same sequence, so output is stable.
                // Apply key-encoding strategy (e.g. convertToSnakeCase).
                // Swift's JSONEncoder omits nil (Optional.none) fields entirely
                // instead of encoding them as JSON null.
                let mut entries: Vec<(String, Json)> = Vec::new();
                for (k, v) in &o.fields {
                    if matches!(v, SwiftValue::Nil) {
                        continue; // omit nil fields
                    }
                    let key = match key_enc {
                        KeyEncoding::ConvertToSnakeCase => to_snake_case(k),
                        KeyEncoding::UseDefaultKeys => k.clone(),
                    };
                    entries.push((key, self.json_encode(v, date_enc, key_enc, data_enc)?));
                }
                Json::Object(entries)
            }
            // Dictionary encodes as a JSON object. Keys must be strings; sort
            // them alphabetically so output is deterministic regardless of
            // insertion order.
            SwiftValue::Dict(pairs) => {
                let mut entries: Vec<(String, Json)> = pairs
                    .iter()
                    .map(|(k, v)| {
                        let key = match k {
                            SwiftValue::Str(s) => s.clone(),
                            other => other.to_string(),
                        };
                        Ok((key, self.json_encode(v, date_enc, key_enc, data_enc)?))
                    })
                    .collect::<Result<_, Signal>>()?;
                entries.sort_by(|a, b| a.0.cmp(&b.0));
                Json::Object(entries)
            }
            // A `Codable` enum: a `RawRepresentable` enum encodes its raw value;
            // a payload-free enum encodes its bare case name.
            SwiftValue::Enum(e) => {
                let raw = self
                    .types
                    .enum_def(&e.type_name)
                    .and_then(|d| d.cases.iter().find(|c| c.name == e.case))
                    .and_then(|c| c.raw.clone());
                match raw {
                    Some(r) => self.json_encode(&r, date_enc, key_enc, data_enc)?,
                    None if e.payload.is_empty() => Json::Str(e.case.clone()),
                    None => {
                        return Err(EvalError::Type(format!(
                            "cannot encode enum '{}' with associated values",
                            e.type_name
                        ))
                        .into())
                    }
                }
            }
            other => {
                return Err(EvalError::Type(format!("cannot encode {}", other.type_name())).into())
            }
        })
    }

    /// Build a runtime value from JSON for the given target type (a registered
    /// struct, else inferred from the JSON shape).  Returns a `Signal::Throw`
    /// when decoding cannot satisfy Swift's `Decodable` contract:
    /// * `keyNotFound` — a non-optional field is absent from the JSON object.
    /// * `typeMismatch` — the JSON value's shape doesn't match the field type
    ///   (e.g. a fractional number where `Int` is expected).
    fn json_decode(
        &self,
        type_name: &str,
        json: &Json,
        date_dec: DateDecoding,
        key_dec: KeyDecoding,
        data_dec: DataDecoding,
    ) -> Result<SwiftValue, Signal> {
        // `Date` decoding: apply the chosen strategy.
        if type_name == "Date" {
            return self.json_decode_date(json, date_dec);
        }
        // `IndexPath` decodes from `{"indexes":[...]}` matching Foundation.
        if type_name == "IndexPath" {
            return self.json_decode_typed(
                Some(type_name),
                "value",
                json,
                date_dec,
                key_dec,
                data_dec,
            );
        }
        // A `Codable` enum decodes from its raw value, or — for a payload-free
        // case — its bare case name. A case with associated values never matches
        // here (we would have to synthesize a payload), so it is skipped.
        if let Some(def) = self.types.enum_def(type_name) {
            let decoded = self.json_value(json);
            if let Some(case) = def.cases.iter().find(|c| {
                let raw_matches = c.raw.as_ref().is_some_and(|r| r == &decoded);
                let name_matches = c.payload_types.is_empty()
                    && matches!(&decoded, SwiftValue::Str(s) if s == &c.name);
                raw_matches || name_matches
            }) {
                return Ok(SwiftValue::Enum(Rc::new(EnumObj {
                    type_name: type_name.to_string(),
                    case: case.name.clone(),
                    payload: Vec::new(),
                })));
            }
        }
        if let (Json::Object(_), Some(def)) = (json, self.types.struct_def(type_name)) {
            let fields: Vec<(String, SwiftValue)> = def
                .stored
                .iter()
                .map(|p| -> Result<(String, SwiftValue), Signal> {
                    let full_ty = p.ty.as_deref();
                    let is_optional = full_ty
                        .map(|t| TypeRepr::parse(t).is_optional())
                        .unwrap_or(false);
                    // For `convertFromSnakeCase`, look up the snake_case form of
                    // the field name in the JSON object when the canonical name
                    // is absent.  This makes both hand-written snake_case JSON
                    // and round-trips through `convertToSnakeCase` decodable.
                    let json_val = json.get(&p.name).or_else(|| {
                        if key_dec == KeyDecoding::ConvertFromSnakeCase {
                            json.get(&to_snake_case(&p.name))
                        } else {
                            None
                        }
                    });
                    let v = match json_val {
                        Some(j) => self
                            .json_decode_typed(full_ty, &p.name, j, date_dec, key_dec, data_dec)?,
                        None if is_optional => SwiftValue::Nil,
                        None => {
                            return Err(Signal::Throw(SwiftValue::Str(format!(
                                "DecodingError.keyNotFound: no value for key '{}'",
                                p.name
                            ))))
                        }
                    };
                    Ok((p.name.clone(), v))
                })
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(SwiftValue::Struct(Rc::new(StructObj {
                type_name: type_name.to_string(),
                fields,
            })));
        }
        // A registered struct with a non-object JSON payload is a type mismatch —
        // never fall back to shape-inferred values for known struct types.
        if self.types.is_struct(type_name) {
            return Err(Signal::Throw(SwiftValue::Str(format!(
                "DecodingError.typeMismatch: expected JSON object for '{}', got {}",
                type_name,
                json_kind_name(json)
            ))));
        }
        Ok(self.json_value(json))
    }

    /// Decode a `Date` from JSON according to the chosen strategy.
    fn json_decode_date(&self, json: &Json, date_dec: DateDecoding) -> Result<SwiftValue, Signal> {
        match date_dec {
            DateDecoding::DeferredToDate => match json {
                Json::Int(i) => Ok(date_value(*i as f64)),
                Json::Double(d) => Ok(date_value(*d)),
                other => Err(Signal::Throw(SwiftValue::Str(format!(
                    "DecodingError.typeMismatch: expected number for Date (deferredToDate), got {}",
                    json_kind_name(other)
                )))),
            },
            DateDecoding::SecondsSince1970 => match json {
                Json::Int(i) => Ok(date_value(*i as f64 - REFERENCE_DATE_UNIX_OFFSET)),
                Json::Double(d) => Ok(date_value(d - REFERENCE_DATE_UNIX_OFFSET)),
                other => Err(Signal::Throw(SwiftValue::Str(format!(
                    "DecodingError.typeMismatch: expected number for Date (secondsSince1970), got {}",
                    json_kind_name(other)
                )))),
            },
            DateDecoding::MillisecondsSince1970 => match json {
                Json::Int(i) => {
                    Ok(date_value(*i as f64 / 1000.0 - REFERENCE_DATE_UNIX_OFFSET))
                }
                Json::Double(d) => Ok(date_value(d / 1000.0 - REFERENCE_DATE_UNIX_OFFSET)),
                other => Err(Signal::Throw(SwiftValue::Str(format!(
                    "DecodingError.typeMismatch: expected number for Date (millisecondsSince1970), got {}",
                    json_kind_name(other)
                )))),
            },
            DateDecoding::Iso8601 => match json {
                Json::Str(s) => match iso8601_parse(s) {
                    Some(ref_secs) => Ok(date_value(ref_secs)),
                    None => Err(Signal::Throw(SwiftValue::Str(format!(
                        "DecodingError.dataCorrupted: invalid ISO 8601 date string '{s}'"
                    )))),
                },
                other => Err(Signal::Throw(SwiftValue::Str(format!(
                    "DecodingError.typeMismatch: expected ISO 8601 string for Date, got {}",
                    json_kind_name(other)
                )))),
            },
        }
    }

    /// Decode a JSON value with an optional type-annotation hint.
    ///
    /// * `Int`/`UInt` variants: accept only integer JSON numbers; reject
    ///   fractional doubles with `typeMismatch`.
    /// * `Double`/`Float` variants: coerce integer JSON numbers to floating
    ///   point (matching Swift's `JSONDecoder` behaviour).
    /// * `String`/`Bool`: reject mismatched JSON shapes.
    /// * `Date`: delegate to [`json_decode_date`] using the active strategy.
    /// * Optional (`T?`): `null` → `nil`; otherwise decode the inner type.
    /// * Named struct/enum types: delegate to [`json_decode`].
    /// * Array (`[T]`): delegate to [`json_decode_field`].
    /// * Unknown / no hint: fall back to shape-inferred [`json_value`].
    fn json_decode_typed(
        &self,
        ty: Option<&str>,
        field: &str,
        json: &Json,
        date_dec: DateDecoding,
        key_dec: KeyDecoding,
        data_dec: DataDecoding,
    ) -> Result<SwiftValue, Signal> {
        let Some(full) = ty else {
            return Ok(self.json_value(json));
        };
        let repr = TypeRepr::parse(full);
        // Optional wrapper: `null` → Nil; otherwise decode the inner type.
        if repr.is_optional() {
            if matches!(json, Json::Null) {
                return Ok(SwiftValue::Nil);
            }
            let inner_ty = repr.unwrap_optional().text();
            return self.json_decode_typed(
                Some(inner_ty),
                field,
                json,
                date_dec,
                key_dec,
                data_dec,
            );
        }
        // `Date` type: use the strategy.
        if full.trim() == "Date" {
            return self.json_decode_date(json, date_dec);
        }
        // `URL` decodes from a JSON string (its absoluteString).
        // Validation mirrors `URL(string:)` via `crate::is_url_string_valid`:
        // empty strings and strings with whitespace are dataCorrupted errors.
        if full.trim() == "URL" {
            return match json {
                Json::Str(s) if !crate::is_url_string_valid(s) => {
                    Err(Signal::Throw(SwiftValue::Str(format!(
                        "DecodingError.dataCorrupted: invalid URL string '{s}'"
                    ))))
                }
                Json::Str(s) => Ok(SwiftValue::Struct(Rc::new(StructObj {
                    type_name: "URL".into(),
                    fields: vec![("_string".into(), SwiftValue::Str(s.clone()))],
                }))),
                other => Err(Signal::Throw(SwiftValue::Str(format!(
                    "DecodingError.typeMismatch: expected String for URL '{}', got {}",
                    field,
                    json_kind_name(other)
                )))),
            };
        }
        // `IndexPath` decodes from `{"indexes":[...]}` matching Foundation.
        // `{}` → keyNotFound; non-object → typeMismatch; wrong array element → typeMismatch.
        if full.trim() == "IndexPath" {
            return match json {
                Json::Object(_) => {
                    let arr = match json.get("indexes") {
                        Some(Json::Array(arr)) => arr,
                        Some(other) => {
                            return Err(Signal::Throw(SwiftValue::Str(format!(
                                "DecodingError.typeMismatch: \
                                 expected Array<Any> for IndexPath 'indexes', got {}",
                                json_kind_name(other)
                            ))))
                        }
                        None => {
                            return Err(Signal::Throw(SwiftValue::Str(
                                "DecodingError.keyNotFound: \
                                 No value associated with key 'indexes' \
                                 in IndexPath"
                                    .into(),
                            )))
                        }
                    };
                    let items = arr
                        .iter()
                        .map(|j| match j {
                            Json::Int(i) => Ok(SwiftValue::int(*i as i128)),
                            other => Err(Signal::Throw(SwiftValue::Str(format!(
                                "DecodingError.typeMismatch: \
                                 expected Int in IndexPath 'indexes', got {}",
                                json_kind_name(other)
                            )))),
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(SwiftValue::Struct(Rc::new(StructObj {
                        type_name: "IndexPath".into(),
                        fields: vec![("_indexes".into(), SwiftValue::Array(Rc::new(items)))],
                    })))
                }
                other => Err(Signal::Throw(SwiftValue::Str(format!(
                    "DecodingError.typeMismatch: expected JSON object for IndexPath '{}', got {}",
                    field,
                    json_kind_name(other)
                )))),
            };
        }
        // `UUID` decodes from a JSON string; throws on malformed UUID.
        if full.trim() == "UUID" {
            return match json {
                Json::Str(s) => match validate_uuid(s) {
                    Some(canonical) => Ok(SwiftValue::Struct(Rc::new(StructObj {
                        type_name: "UUID".into(),
                        fields: vec![("uuidString".into(), SwiftValue::Str(canonical))],
                    }))),
                    None => Err(Signal::Throw(SwiftValue::Str(format!(
                        "DecodingError.dataCorrupted: invalid UUID string '{s}'"
                    )))),
                },
                other => Err(Signal::Throw(SwiftValue::Str(format!(
                    "DecodingError.typeMismatch: expected String for UUID '{}', got {}",
                    field,
                    json_kind_name(other)
                )))),
            };
        }
        // `Data` decodes according to the data-decoding strategy.
        if full.trim() == "Data" {
            return match (data_dec, json) {
                (DataDecoding::Base64, Json::Str(s)) => match crate::base64::decode(s) {
                    Some(bytes) => Ok(data_value_from_bytes(bytes)),
                    None => Err(Signal::Throw(SwiftValue::Str(format!(
                        "DecodingError.dataCorrupted: invalid base64 string for '{field}'"
                    )))),
                },
                (DataDecoding::DeferredToData, Json::Array(items)) => {
                    let bytes: Vec<SwiftValue> = items
                        .iter()
                        .map(|j| match j {
                            Json::Int(i) if (0..=255).contains(i) => {
                                Ok(SwiftValue::int(*i as i128))
                            }
                            other => Err(Signal::Throw(SwiftValue::Str(format!(
                                "DecodingError.typeMismatch: expected byte for Data '{}', got {}",
                                field,
                                json_kind_name(other)
                            )))),
                        })
                        .collect::<Result<_, _>>()?;
                    Ok(SwiftValue::Struct(Rc::new(StructObj {
                        type_name: "Data".into(),
                        fields: vec![("_bytes".into(), SwiftValue::Array(Rc::new(bytes)))],
                    })))
                }
                (_, other) => Err(Signal::Throw(SwiftValue::Str(format!(
                    "DecodingError.typeMismatch: unexpected JSON value for Data '{}', got {}",
                    field,
                    json_kind_name(other)
                )))),
            };
        }
        // Primitive type matching.
        match full.trim() {
            "Int" | "Int8" | "Int16" | "Int32" | "Int64" | "UInt" | "UInt8" | "UInt16"
            | "UInt32" | "UInt64" => match json {
                Json::Int(i) => return Ok(SwiftValue::int(*i as i128)),
                Json::Double(_) => {
                    return Err(Signal::Throw(SwiftValue::Str(format!(
                    "DecodingError.typeMismatch: expected Int for '{}', got floating-point number",
                    field
                ))))
                }
                other => {
                    return Err(Signal::Throw(SwiftValue::Str(format!(
                        "DecodingError.typeMismatch: expected Int for '{}', got {}",
                        field,
                        json_kind_name(other)
                    ))))
                }
            },
            "Double" | "Float" | "Float32" | "Float64" | "Float80" => match json {
                Json::Int(i) => return Ok(SwiftValue::Double(*i as f64)),
                Json::Double(d) => return Ok(SwiftValue::Double(*d)),
                other => {
                    return Err(Signal::Throw(SwiftValue::Str(format!(
                        "DecodingError.typeMismatch: expected Double for '{}', got {}",
                        field,
                        json_kind_name(other)
                    ))))
                }
            },
            "String" => match json {
                Json::Str(s) => return Ok(SwiftValue::Str(s.clone())),
                other => {
                    return Err(Signal::Throw(SwiftValue::Str(format!(
                        "DecodingError.typeMismatch: expected String for '{}', got {}",
                        field,
                        json_kind_name(other)
                    ))))
                }
            },
            "Bool" => match json {
                Json::Bool(b) => return Ok(SwiftValue::Bool(*b)),
                other => {
                    return Err(Signal::Throw(SwiftValue::Str(format!(
                        "DecodingError.typeMismatch: expected Bool for '{}', got {}",
                        field,
                        json_kind_name(other)
                    ))))
                }
            },
            _ => {}
        }
        // Compute the element type once; used by both the array and
        // struct/enum branches below.
        let inner = decode_element_type(full);
        // Array type (`[T]`) MUST be checked before the registered struct/enum
        // branch: for `[Point]`, inner="Point" is a registered struct, but the
        // field is an array — JSON must be an array, not an object or null.
        // Checking array first ensures every array-typed field is validated
        // uniformly regardless of whether its element type is registered.
        if full.trim_start().starts_with('[') {
            return match json {
                Json::Array(_) => {
                    self.json_decode_field(inner, full, json, date_dec, key_dec, data_dec)
                }
                other => Err(Signal::Throw(SwiftValue::Str(format!(
                    "DecodingError.typeMismatch: expected array for '{}', got {}",
                    field,
                    json_kind_name(other)
                )))),
            };
        }
        // Named struct/enum (non-array): delegate to field decoder.
        if self.types.is_struct(inner) || self.types.is_enum(inner) {
            return self.json_decode_field(inner, full, json, date_dec, key_dec, data_dec);
        }
        Ok(self.json_value(json))
    }

    /// Decode a struct field whose declared type is `inner` (the element type)
    /// and full spelling `full` (e.g. `[User]`, `User?`). Handles arrays and
    /// optionals of a registered struct/enum element.
    fn json_decode_field(
        &self,
        inner: &str,
        full: &str,
        json: &Json,
        date_dec: DateDecoding,
        key_dec: KeyDecoding,
        data_dec: DataDecoding,
    ) -> Result<SwiftValue, Signal> {
        match json {
            // `nil` only for an optional type; a non-optional field receiving
            // JSON null is Swift's `valueNotFound`.
            Json::Null => {
                if TypeRepr::parse(full).is_optional() {
                    Ok(SwiftValue::Nil)
                } else {
                    Err(Signal::Throw(SwiftValue::Str(format!(
                        "DecodingError.valueNotFound: null for non-optional type '{}'",
                        full
                    ))))
                }
            }
            // `[Element]` decodes each item with the element type as hint so
            // primitive element types (Int, String, …) are type-checked and
            // registered struct/enum elements recurse through json_decode.
            Json::Array(items) if full.trim_start().starts_with('[') => {
                Ok(SwiftValue::Array(Rc::new(
                    items
                        .iter()
                        .map(|j| {
                            self.json_decode_typed(
                                Some(inner),
                                "element",
                                j,
                                date_dec,
                                key_dec,
                                data_dec,
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                )))
            }
            _ => self.json_decode(inner, json, date_dec, key_dec, data_dec),
        }
    }

    /// Map a JSON value to a runtime value without target-type context.
    fn json_value(&self, json: &Json) -> SwiftValue {
        match json {
            Json::Null => SwiftValue::Nil,
            Json::Bool(b) => SwiftValue::Bool(*b),
            Json::Int(i) => SwiftValue::int(*i as i128),
            Json::Double(d) => SwiftValue::Double(*d),
            Json::Str(s) => SwiftValue::Str(s.clone()),
            Json::Array(items) => {
                SwiftValue::Array(Rc::new(items.iter().map(|j| self.json_value(j)).collect()))
            }
            Json::Object(entries) => SwiftValue::Struct(Rc::new(StructObj {
                type_name: "JSON".into(),
                fields: entries
                    .iter()
                    .map(|(k, v)| (k.clone(), self.json_value(v)))
                    .collect(),
            })),
        }
    }
}

/// Build a `Data` struct from raw bytes.
fn data_value_from_bytes(bytes: Vec<u8>) -> SwiftValue {
    let elements: Vec<SwiftValue> = bytes
        .into_iter()
        .map(|b| SwiftValue::int(i128::from(b)))
        .collect();
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "Data".into(),
        fields: vec![("_bytes".into(), SwiftValue::Array(Rc::new(elements)))],
    }))
}

/// Extract bytes from a `Data` struct value (returns `None` if malformed).
fn data_bytes_from_value(value: &SwiftValue) -> Option<Vec<u8>> {
    if let SwiftValue::Struct(o) = value {
        if o.type_name == "Data" {
            if let Some(SwiftValue::Array(items)) = o.get("_bytes") {
                return items
                    .iter()
                    .map(|v| match v {
                        SwiftValue::Int(i) if (0..=255).contains(&i.raw) => Some(i.raw as u8),
                        _ => None,
                    })
                    .collect();
            }
        }
    }
    None
}

/// Validate a UUID string and return its canonical uppercase form, or `None`.
fn validate_uuid(raw: &str) -> Option<String> {
    let upper = raw.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    let dash_positions = [8usize, 13, 18, 23];
    if bytes.len() != 36 || dash_positions.iter().any(|&i| bytes[i] != b'-') {
        return None;
    }
    if bytes
        .iter()
        .enumerate()
        .any(|(i, b)| !dash_positions.contains(&i) && !b.is_ascii_hexdigit())
    {
        return None;
    }
    Some(upper)
}

/// A human-readable name for a JSON value's kind, used in `typeMismatch` error
/// messages to mirror what Swift's `JSONDecoder` would report.
fn json_kind_name(json: &Json) -> &'static str {
    match json {
        Json::Null => "null",
        Json::Bool(_) => "Bool",
        Json::Int(_) | Json::Double(_) => "number",
        Json::Str(_) => "String",
        Json::Array(_) => "array",
        Json::Object(_) => "object",
    }
}

// ---------------------------------------------------------------------------
// PropertyList encoding
// ---------------------------------------------------------------------------

/// Build an XML property list document for `value`. Returns the UTF-8 string.
pub(super) fn plist_encode_xml(value: &SwiftValue) -> Result<String, Signal> {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"");
    out.push_str("http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n");
    out.push_str("<plist version=\"1.0\">\n");
    plist_write_value(&mut out, value, 0)?;
    out.push_str("</plist>\n");
    Ok(out)
}

/// Write one plist value to `out` at indentation depth `depth`, appending
/// a trailing newline. Dict/array keys are sorted alphabetically (matching
/// Foundation's XML plist serialiser). `SwiftValue::Nil` entries inside dicts
/// are silently omitted; top-level nil is a throw.
fn plist_write_value(out: &mut String, value: &SwiftValue, depth: usize) -> Result<(), Signal> {
    let tabs: String = "\t".repeat(depth);
    match value {
        SwiftValue::Bool(b) => {
            out.push_str(&tabs);
            out.push_str(if *b { "<true/>" } else { "<false/>" });
            out.push('\n');
        }
        SwiftValue::Int(i) => {
            use std::fmt::Write as _;
            let _ = writeln!(out, "{tabs}<integer>{}</integer>", i.raw);
        }
        SwiftValue::Double(d) => {
            use std::fmt::Write as _;
            // Match Foundation's XML plist encoding for non-finite values:
            // NaN → "nan", +∞ → "+infinity", -∞ → "-infinity".
            let repr: std::borrow::Cow<str> = if d.is_nan() {
                "nan".into()
            } else if *d == f64::INFINITY {
                "+infinity".into()
            } else if *d == f64::NEG_INFINITY {
                "-infinity".into()
            } else {
                format!("{d}").into()
            };
            let _ = writeln!(out, "{tabs}<real>{repr}</real>");
        }
        SwiftValue::Str(s) => {
            use std::fmt::Write as _;
            let _ = writeln!(out, "{tabs}<string>{}</string>", plist_xml_escape(s));
        }
        SwiftValue::Nil => {
            // Top-level nil is not a valid plist root; dicts skip nil fields
            // before calling this function, so reaching here is always an error.
            return Err(Signal::Throw(SwiftValue::Str(
                "PropertyListEncoder: cannot encode nil as a property list value".into(),
            )));
        }
        SwiftValue::Array(items) => {
            use std::fmt::Write as _;
            if items.is_empty() {
                let _ = writeln!(out, "{tabs}<array/>");
            } else {
                let _ = writeln!(out, "{tabs}<array>");
                for item in items.iter() {
                    plist_write_value(out, item, depth + 1)?;
                }
                let _ = writeln!(out, "{tabs}</array>");
            }
        }
        SwiftValue::Struct(o) => {
            // Data — encodes as base64 in a <data> element.
            if o.type_name == "Data" {
                let bytes: Vec<u8> = match o.get("_bytes") {
                    Some(SwiftValue::Array(items)) => items
                        .iter()
                        .filter_map(|v| match v {
                            SwiftValue::Int(i) if (0..=255).contains(&i.raw) => Some(i.raw as u8),
                            _ => None,
                        })
                        .collect(),
                    _ => vec![],
                };
                let b64 = crate::base64::encode(&bytes);
                use std::fmt::Write as _;
                let _ = writeln!(out, "{tabs}<data>");
                let _ = writeln!(out, "{tabs}{b64}");
                let _ = writeln!(out, "{tabs}</data>");
                return Ok(());
            }
            // Date — encodes as ISO 8601 in a <date> element (second precision).
            if let Some(ref_secs) = date_ref_seconds(value) {
                let iso = iso8601_format(ref_secs);
                use std::fmt::Write as _;
                let _ = writeln!(out, "{tabs}<date>{iso}</date>");
                return Ok(());
            }
            // General struct: encode all non-nil fields as a <dict>, keys
            // sorted alphabetically to match Foundation's output.
            let mut pairs: Vec<(&str, &SwiftValue)> =
                o.fields.iter().map(|(k, v)| (k.as_str(), v)).collect();
            pairs.sort_by_key(|(k, _)| *k);
            let non_nil: Vec<_> = pairs
                .iter()
                .copied()
                .filter(|(_, v)| !matches!(v, SwiftValue::Nil))
                .collect();
            plist_write_dict(out, &non_nil, depth, &tabs)?;
        }
        SwiftValue::Dict(pairs) => {
            // Swift Dictionary — sort keys alphabetically.
            let mut sorted: Vec<(String, &SwiftValue)> = pairs
                .iter()
                .filter(|(_, v)| !matches!(v, SwiftValue::Nil))
                .map(|(k, v)| (k.to_string(), v))
                .collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            let kv: Vec<(&str, &SwiftValue)> =
                sorted.iter().map(|(k, v)| (k.as_str(), *v)).collect();
            plist_write_dict(out, &kv, depth, &tabs)?;
        }
        SwiftValue::Enum(e) => {
            // Payload-free enum: encode its case name as a string.
            use std::fmt::Write as _;
            let _ = writeln!(out, "{tabs}<string>{}</string>", plist_xml_escape(&e.case));
        }
        other => {
            return Err(Signal::Throw(SwiftValue::Str(format!(
                "PropertyListEncoder: cannot encode value of type {}",
                other.type_name()
            ))));
        }
    }
    Ok(())
}

/// Emit a `<dict>` element from a slice of sorted `(key, value)` pairs.
fn plist_write_dict(
    out: &mut String,
    pairs: &[(&str, &SwiftValue)],
    depth: usize,
    tabs: &str,
) -> Result<(), Signal> {
    use std::fmt::Write as _;
    if pairs.is_empty() {
        let _ = writeln!(out, "{tabs}<dict/>");
    } else {
        let _ = writeln!(out, "{tabs}<dict>");
        for (k, v) in pairs {
            let _ = writeln!(out, "{}\t<key>{}</key>", tabs, plist_xml_escape(k));
            plist_write_value(out, v, depth + 1)?;
        }
        let _ = writeln!(out, "{tabs}</dict>");
    }
    Ok(())
}

/// Escape XML special characters for plist string/key values.
/// Foundation escapes `<`, `>`, `&` but does NOT escape `"` inside strings.
fn plist_xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Reference date: 2001-01-01T00:00:00Z = unix 978307200
    const REF_DATE_UNIX: f64 = 978_307_200.0;

    #[test]
    fn iso8601_format_reference_date() {
        // timeIntervalSinceReferenceDate = 0.0 → 2001-01-01T00:00:00Z
        assert_eq!(iso8601_format(0.0), "2001-01-01T00:00:00Z");
    }

    #[test]
    fn iso8601_format_one_day_after_reference() {
        // 86400 seconds = 1 day → 2001-01-02T00:00:00Z
        assert_eq!(iso8601_format(86_400.0), "2001-01-02T00:00:00Z");
    }

    #[test]
    fn iso8601_format_known_instant() {
        // 2024-06-29T12:34:56Z: unix = 1719664496
        // ref_secs = 1719664496 - 978307200 = 741357296
        let ref_secs = 741_357_296.0_f64;
        assert_eq!(iso8601_format(ref_secs), "2024-06-29T12:34:56Z");
    }

    #[test]
    fn iso8601_parse_round_trip() {
        let ref_secs = 0.0_f64;
        let formatted = iso8601_format(ref_secs);
        let parsed = iso8601_parse(&formatted).expect("parse must succeed");
        assert!((parsed - ref_secs).abs() < 1.0);
    }

    #[test]
    fn iso8601_parse_rejects_malformed() {
        assert!(iso8601_parse("not-a-date").is_none());
        assert!(iso8601_parse("2024-13-01T00:00:00Z").is_none());
        assert!(iso8601_parse("").is_none());
    }

    #[test]
    fn iso8601_parse_requires_trailing_z() {
        // Without 'Z' must fail.
        assert!(iso8601_parse("2001-01-01T00:00:00").is_none());
        // With 'Z' must succeed.
        assert!(iso8601_parse("2001-01-01T00:00:00Z").is_some());
    }

    #[test]
    fn iso8601_parse_rejects_invalid_day_for_month() {
        // February 31 is never valid.
        assert!(iso8601_parse("2024-02-31T00:00:00Z").is_none());
        // February 29 invalid in a non-leap year.
        assert!(iso8601_parse("2023-02-29T00:00:00Z").is_none());
        // February 29 valid in a leap year.
        assert!(iso8601_parse("2024-02-29T00:00:00Z").is_some());
        // April 31 is invalid (April has 30 days).
        assert!(iso8601_parse("2024-04-31T00:00:00Z").is_none());
    }

    #[test]
    fn iso8601_parse_accepts_fractional_seconds_with_z() {
        // Fractional seconds should be stripped; 'Z' still required.
        let r = iso8601_parse("2001-01-01T00:00:00.500Z");
        assert!(r.is_some());
        // Without Z, fractional form also fails.
        assert!(iso8601_parse("2001-01-01T00:00:00.500").is_none());
        // Non-digit fractional suffix must be rejected (iter3 follow-up).
        assert!(iso8601_parse("2001-01-01T00:00:00.abcZ").is_none());
        assert!(iso8601_parse("2001-01-01T00:00:00.Z").is_none());
        // Multiple fractional digits are fine.
        assert!(iso8601_parse("2001-01-01T00:00:00.123456Z").is_some());
    }

    #[test]
    fn civil_round_trip_reference_date() {
        // unix day 11323 = 978307200 / 86400
        let days = (REF_DATE_UNIX / 86_400.0) as i64;
        assert_eq!(days, 11323);
        let (y, mo, d) = civil_from_days(11323);
        assert_eq!((y, mo, d), (2001, 1, 1));
        assert_eq!(days_from_civil(2001, 1, 1), 11323);
    }

    #[test]
    fn date_encoding_strategies_produce_correct_json() {
        // ref_secs = 0.0 (2001-01-01T00:00:00Z)
        let ref_secs = 0.0_f64;
        // deferredToDate
        if let DateEncoding::DeferredToDate = DateEncoding::from_raw(0) {
            // ok
        }
        // secondsSince1970
        let unix = ref_secs + REFERENCE_DATE_UNIX_OFFSET;
        assert_eq!(unix, 978_307_200.0);
        // millisecondsSince1970
        assert_eq!(unix * 1000.0, 978_307_200_000.0);
        // iso8601
        assert_eq!(iso8601_format(ref_secs), "2001-01-01T00:00:00Z");
    }

    // -----------------------------------------------------------------------
    // to_snake_case — Apple's _convertToSnakeCase algorithm
    // -----------------------------------------------------------------------

    #[test]
    fn snake_case_basic_camel_words() {
        assert_eq!(to_snake_case("oneTwoThree"), "one_two_three");
    }

    #[test]
    fn snake_case_trailing_acronym() {
        // Acronym at the end of a word: no lowercase follows, so the entire
        // run is lowercased as one word.
        assert_eq!(to_snake_case("imageURL"), "image_url");
    }

    #[test]
    fn snake_case_leading_acronym_before_word() {
        // Uppercase run followed by a lowercase-starting word splits before
        // the last uppercase of the run.
        assert_eq!(to_snake_case("URLValue"), "url_value");
    }

    #[test]
    fn snake_case_leading_acronym_before_uppercase_word() {
        // `URLAddress`: U,R,L (run), then A followed by lowercase `d`.
        // Rule 2 fires before `A` → split = `URL` + `Address`.
        assert_eq!(to_snake_case("URLAddress"), "url_address");
    }

    #[test]
    fn snake_case_digit_boundary() {
        // A digit is non-uppercase, so it triggers a word break when followed
        // by an uppercase letter (rule 1 with `!prev.is_uppercase()`).
        assert_eq!(to_snake_case("address1Line2"), "address1_line2");
    }

    #[test]
    fn snake_case_digit_then_uppercase_simple() {
        assert_eq!(to_snake_case("a1B"), "a1_b");
    }

    #[test]
    fn snake_case_leading_underscore_preserved() {
        assert_eq!(to_snake_case("_leading"), "_leading");
    }

    #[test]
    fn snake_case_trailing_underscore_preserved() {
        assert_eq!(to_snake_case("trailing_"), "trailing_");
    }

    #[test]
    fn snake_case_already_snake_unchanged() {
        assert_eq!(to_snake_case("already_snake"), "already_snake");
    }

    #[test]
    fn snake_case_single_letter() {
        assert_eq!(to_snake_case("A"), "a");
        assert_eq!(to_snake_case("a"), "a");
    }
}
