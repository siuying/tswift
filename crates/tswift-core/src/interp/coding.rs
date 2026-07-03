//! `Codable` support: JSON encoding/decoding for the `JSONEncoder`/`JSONDecoder`
//! markers, layered over the hand-written [`crate::json`] model.

use std::rc::Rc;

use tswift_frontend::{Node, TypeRepr};

use super::{decode_element_type, metatype_name, EvalError, Interpreter, Signal};
use crate::json::Json;
use crate::value::{EnumObj, StructObj, SwiftValue};

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
            let args = self.eval_args(arg_nodes)?;
            let value = args
                .first()
                .map(|a| a.value.clone())
                .ok_or_else(|| EvalError::Type("encode expects a value".into()))?;
            let json = self.json_encode(&value)?;
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
            return Ok(Some(self.json_decode(&type_name, &json)?));
        }
        Ok(None)
    }

    /// Serialize a `Codable` value to its `JSONEncoder` representation.
    fn json_encode(&self, value: &SwiftValue) -> Result<Json, Signal> {
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
                    .map(|v| self.json_encode(v))
                    .collect::<Result<_, _>>()?,
            ),
            SwiftValue::Struct(o) => {
                // Encode fields in declaration order — the parser always
                // produces them in the same sequence, so output is stable.
                let entries: Vec<(String, Json)> = o
                    .fields
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), self.json_encode(v)?)))
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
                        Ok((key, self.json_encode(v)?))
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
                    Some(r) => self.json_encode(&r)?,
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
    fn json_decode(&self, type_name: &str, json: &Json) -> Result<SwiftValue, Signal> {
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
                        Some(j) => self.json_decode_typed(full_ty, &p.name, j)?,
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

    /// Decode a JSON value with an optional type-annotation hint.
    ///
    /// * `Int`/`UInt` variants: accept only integer JSON numbers; reject
    ///   fractional doubles with `typeMismatch`.
    /// * `Double`/`Float` variants: coerce integer JSON numbers to floating
    ///   point (matching Swift's `JSONDecoder` behaviour).
    /// * `String`/`Bool`: reject mismatched JSON shapes.
    /// * Optional (`T?`): `null` → `nil`; otherwise decode the inner type.
    /// * Named struct/enum types: delegate to [`json_decode`].
    /// * Array (`[T]`): delegate to [`json_decode_field`].
    /// * Unknown / no hint: fall back to shape-inferred [`json_value`].
    fn json_decode_typed(
        &self,
        ty: Option<&str>,
        field: &str,
        json: &Json,
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
            return self.json_decode_typed(Some(inner_ty), field, json);
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
                Json::Array(_) => self.json_decode_field(inner, full, json),
                other => Err(Signal::Throw(SwiftValue::Str(format!(
                    "DecodingError.typeMismatch: expected array for '{}', got {}",
                    field,
                    json_kind_name(other)
                )))),
            };
        }
        // Named struct/enum (non-array): delegate to field decoder.
        if self.types.is_struct(inner) || self.types.is_enum(inner) {
            return self.json_decode_field(inner, full, json);
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
                        .map(|j| self.json_decode_typed(Some(inner), "element", j))
                        .collect::<Result<Vec<_>, _>>()?,
                )))
            }
            _ => self.json_decode(inner, json),
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
