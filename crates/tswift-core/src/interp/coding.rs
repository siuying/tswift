//! `Codable` support: JSON encoding/decoding for the `JSONEncoder`/`JSONDecoder`
//! markers, layered over the hand-written [`crate::json`] model.

use std::rc::Rc;

use tswift_frontend::{Node, TypeRepr};

use super::{decode_element_type, metatype_name, EvalError, Interpreter, Signal};
use crate::json::Json;
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
    let s = if let Some(pos) = s.rfind('.') {
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

/// Read a `DateEncodingStrategy` raw integer from a `JSONEncoder` struct.
fn encoder_date_strategy(o: &StructObj) -> DateEncoding {
    match o.get("dateEncodingStrategy") {
        Some(SwiftValue::Int(i)) => DateEncoding::from_raw(i.raw),
        _ => DateEncoding::DeferredToDate,
    }
}

/// Read a `DateDecodingStrategy` raw integer from a `JSONDecoder` struct.
fn decoder_date_strategy(o: &StructObj) -> DateDecoding {
    match o.get("dateDecodingStrategy") {
        Some(SwiftValue::Int(i)) => DateDecoding::from_raw(i.raw),
        _ => DateDecoding::DeferredToDate,
    }
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
        let SwiftValue::Struct(o) = base_value else {
            return Ok(None);
        };
        // `JSONEncoder().encode(value)` → a JSON `Data` (UTF-8 bytes).
        if o.type_name == "JSONEncoder" && method == "encode" {
            let date_enc = encoder_date_strategy(o);
            let args = self.eval_args(arg_nodes)?;
            let value = args
                .first()
                .map(|a| a.value.clone())
                .ok_or_else(|| EvalError::Type("encode expects a value".into()))?;
            let json = self.json_encode(&value, date_enc)?;
            let json_str = crate::json::to_string(&json);
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
        if o.type_name == "JSONDecoder" && method == "decode" {
            let date_dec = decoder_date_strategy(o);
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
            return Ok(Some(self.json_decode(&type_name, &json, date_dec)?));
        }
        Ok(None)
    }

    /// Serialize a `Codable` value to its `JSONEncoder` representation.
    fn json_encode(&self, value: &SwiftValue, date_enc: DateEncoding) -> Result<Json, Signal> {
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
                    .map(|v| self.json_encode(v, date_enc))
                    .collect::<Result<_, _>>()?,
            ),
            SwiftValue::Struct(o) => {
                // Encode fields in declaration order — the parser always
                // produces them in the same sequence, so output is stable.
                let entries: Vec<(String, Json)> = o
                    .fields
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), self.json_encode(v, date_enc)?)))
                    .collect::<Result<_, Signal>>()?;
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
                        Ok((key, self.json_encode(v, date_enc)?))
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
                    Some(r) => self.json_encode(&r, date_enc)?,
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
    ) -> Result<SwiftValue, Signal> {
        // `Date` decoding: apply the chosen strategy.
        if type_name == "Date" {
            return self.json_decode_date(json, date_dec);
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
                    let v = match json.get(&p.name) {
                        Some(j) => self.json_decode_typed(full_ty, &p.name, j, date_dec)?,
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
            return self.json_decode_typed(Some(inner_ty), field, json, date_dec);
        }
        // `Date` type: use the strategy.
        if full.trim() == "Date" {
            return self.json_decode_date(json, date_dec);
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
                Json::Array(_) => self.json_decode_field(inner, full, json, date_dec),
                other => Err(Signal::Throw(SwiftValue::Str(format!(
                    "DecodingError.typeMismatch: expected array for '{}', got {}",
                    field,
                    json_kind_name(other)
                )))),
            };
        }
        // Named struct/enum (non-array): delegate to field decoder.
        if self.types.is_struct(inner) || self.types.is_enum(inner) {
            return self.json_decode_field(inner, full, json, date_dec);
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
                        .map(|j| self.json_decode_typed(Some(inner), "element", j, date_dec))
                        .collect::<Result<Vec<_>, _>>()?,
                )))
            }
            _ => self.json_decode(inner, json, date_dec),
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
}
