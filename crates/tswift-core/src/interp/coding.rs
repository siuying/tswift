//! `Codable` support: JSON encoding/decoding for the `JSONEncoder`/`JSONDecoder`
//! markers, layered over the hand-written [`crate::json`] model.

use std::rc::Rc;

use tswift_frontend::Node;

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
        // `JSONEncoder().encode(value)` → a JSON `Data` (modeled as a String).
        if o.type_name == "JSONEncoder" && method == "encode" {
            let args = self.eval_args(arg_nodes)?;
            let value = args
                .first()
                .map(|a| a.value.clone())
                .ok_or_else(|| EvalError::Type("encode expects a value".into()))?;
            let json = self.json_encode(&value)?;
            return Ok(Some(SwiftValue::Str(crate::json::to_string(&json))));
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
            return Ok(Some(self.json_decode(&type_name, &json)));
        }
        Ok(None)
    }

    /// Serialize a `Codable` value to its `JSONEncoder` representation.
    fn json_encode(&self, value: &SwiftValue) -> Result<Json, Signal> {
        Ok(match value {
            SwiftValue::Nil => Json::Null,
            SwiftValue::Bool(b) => Json::Bool(*b),
            SwiftValue::Int(i) => Json::Int(i.raw as i64),
            SwiftValue::Double(d) => Json::Double(*d),
            SwiftValue::Str(s) => Json::Str(s.clone()),
            SwiftValue::Array(items) => Json::Array(
                items
                    .iter()
                    .map(|v| self.json_encode(v))
                    .collect::<Result<_, _>>()?,
            ),
            SwiftValue::Struct(o) => Json::Object(
                o.fields
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), self.json_encode(v)?)))
                    .collect::<Result<_, Signal>>()?,
            ),
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
    /// struct, else inferred from the JSON shape).
    fn json_decode(&self, type_name: &str, json: &Json) -> SwiftValue {
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
                return SwiftValue::Enum(Rc::new(EnumObj {
                    type_name: type_name.to_string(),
                    case: case.name.clone(),
                    payload: Vec::new(),
                }));
            }
        }
        if let (Json::Object(_), Some(def)) = (json, self.types.struct_def(type_name)) {
            let fields: Vec<(String, SwiftValue)> = def
                .stored
                .iter()
                .map(|p| {
                    let v = json
                        .get(&p.name)
                        .map(|j| {
                            // Decode typed nested fields (structs/enums) by their
                            // declared element type so they round-trip; fall back
                            // to a shape-inferred value otherwise.
                            match p.ty.as_deref() {
                                Some(full)
                                    if self.types.is_struct(decode_element_type(full))
                                        || self.types.is_enum(decode_element_type(full)) =>
                                {
                                    self.json_decode_field(decode_element_type(full), full, j)
                                }
                                _ => self.json_value(j),
                            }
                        })
                        .unwrap_or(SwiftValue::Nil);
                    (p.name.clone(), v)
                })
                .collect();
            return SwiftValue::Struct(Rc::new(StructObj {
                type_name: type_name.to_string(),
                fields,
            }));
        }
        self.json_value(json)
    }

    /// Decode a struct field whose declared type is `inner` (the element type)
    /// and full spelling `full` (e.g. `[User]`, `User?`). Handles arrays and
    /// optionals of a registered struct/enum element.
    fn json_decode_field(&self, inner: &str, full: &str, json: &Json) -> SwiftValue {
        match json {
            // `nil` for an absent optional.
            Json::Null => SwiftValue::Nil,
            // `[Element]` decodes each item by the element type.
            Json::Array(items) if full.trim_start().starts_with('[') => SwiftValue::Array(Rc::new(
                items.iter().map(|j| self.json_decode(inner, j)).collect(),
            )),
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
