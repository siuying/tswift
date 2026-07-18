//! The host-native function bridge (Epic #246, slice #248).
//!
//! A *host function* is a Rust/native function that interpreted Swift can call
//! by name, crossing the interpreter boundary through a JSON wire. The design
//! mirrors the HTTP transport seam ([`crate::http`]): the interpreter stays a
//! cooperative single-threaded executor (ADR-0005), so the call is
//! **synchronous** — the interpreter evaluates and validates the arguments,
//! encodes them to JSON, hands them to a [`HostCallHandler`], then decodes and
//! validates the returned JSON against the declared return type.
//!
//! This is stage 1: *fixed-type* functions only. The supported type vocabulary
//! is `Void`, `Bool`, `Int`, `Double`, `String`, optionals, arrays, and
//! string-keyed dictionaries — the shapes that map cleanly onto the small JSON
//! layer in [`crate::json`] without a new dependency.
//!
//! One shared trampoline (`Interpreter::call_host_fn`) serves every registered
//! host function; there is no per-function codegen. Registration installs a
//! name into the interpreter's existing free-function dispatch path, and the
//! trampoline does the encode / call / decode dance.

use crate::json::{self, Json};
use crate::value::{IntValue, SwiftValue};
use std::collections::HashMap;
use std::sync::Arc;

/// The host-side callback invoked when interpreted Swift calls a registered
/// host function.
///
/// `args_json` is a JSON **array** of the call's already-validated arguments in
/// declared order (each argument encoded per its declared parameter type). The
/// handler returns either:
///
/// - `Ok(json)` — a JSON document decoded against the function's return type
///   (`"null"` / any value for a `Void` return), OR a `{"$thrown": <message>}`
///   object to raise a catchable Swift error; or
/// - `Err(message)` — a host-side failure surfaced as an interpreter error
///   (not a Swift-catchable error) naming the function.
///
/// `Send + Sync` so an embedding may share one handler across threads; the
/// interpreter itself calls it on a single thread.
pub trait HostCallHandler: Send + Sync {
    /// Invoke host function `name` with `args_json` (a JSON array of args).
    fn call(&self, name: &str, args_json: &str) -> Result<String, String>;
}

/// A stage-1 fixed type in a host-function signature.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    Void,
    Bool,
    Int,
    Double,
    String,
    /// `T?` — a present value or `nil`.
    Optional(Box<TypeExpr>),
    /// `[T]`.
    Array(Box<TypeExpr>),
    /// `[String: V]` — string-keyed dictionary.
    Dictionary(Box<TypeExpr>),
}

impl TypeExpr {
    /// Parse a [`TypeExpr`] from its compact JSON encoding.
    ///
    /// Scalars are bare strings (`"Int"`, `"String"`, …); compound types are
    /// single-key objects: `{"optional": T}`, `{"array": T}`,
    /// `{"dictionary": V}` (keys are always `String`).
    pub fn from_json(node: &Json) -> Result<TypeExpr, String> {
        match node {
            Json::Str(s) => match s.as_str() {
                "Void" => Ok(TypeExpr::Void),
                "Bool" => Ok(TypeExpr::Bool),
                "Int" => Ok(TypeExpr::Int),
                "Double" => Ok(TypeExpr::Double),
                "String" => Ok(TypeExpr::String),
                other => Err(format!("unknown host type `{other}`")),
            },
            Json::Object(entries) => {
                let (key, val) = entries
                    .first()
                    .ok_or_else(|| "empty type object".to_string())?;
                if entries.len() != 1 {
                    return Err("compound type must have exactly one key".into());
                }
                match key.as_str() {
                    "optional" => Ok(TypeExpr::Optional(Box::new(TypeExpr::from_json(val)?))),
                    "array" => Ok(TypeExpr::Array(Box::new(TypeExpr::from_json(val)?))),
                    "dictionary" => Ok(TypeExpr::Dictionary(Box::new(TypeExpr::from_json(val)?))),
                    other => Err(format!("unknown compound host type `{other}`")),
                }
            }
            _ => Err("type must be a string or single-key object".into()),
        }
    }
}

/// One parameter of a host-function signature: an external label and a type.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    /// The external argument label. Empty means the argument is unlabelled
    /// (Swift's `_`).
    pub label: String,
    pub ty: TypeExpr,
}

/// A parsed host-function signature.
#[derive(Debug, Clone, PartialEq)]
pub struct Signature {
    pub name: String,
    pub params: Vec<Param>,
    pub returns: TypeExpr,
    pub throws: bool,
}

impl Signature {
    /// Parse a [`Signature`] from its JSON schema:
    ///
    /// ```json
    /// {"name": "greet",
    ///  "params": [{"label": "name", "type": "String"}],
    ///  "returns": "String",
    ///  "throws": false}
    /// ```
    ///
    /// `params` defaults to empty, `returns` to `"Void"`, `throws` to `false`.
    pub fn from_json(text: &str) -> Result<Signature, String> {
        let root = json::parse(text).map_err(|e| format!("signature JSON parse error: {e}"))?;
        let name = match root.get("name") {
            Some(Json::Str(s)) if !s.is_empty() => s.clone(),
            _ => return Err("signature needs a non-empty string `name`".into()),
        };
        let mut params = Vec::new();
        if let Some(node) = root.get("params") {
            let Json::Array(items) = node else {
                return Err("signature `params` must be an array".into());
            };
            for item in items {
                let label = match item.get("label") {
                    Some(Json::Str(s)) => s.clone(),
                    None => String::new(),
                    _ => return Err("param `label` must be a string".into()),
                };
                let ty = match item.get("type") {
                    Some(t) => TypeExpr::from_json(t)?,
                    None => return Err("param needs a `type`".into()),
                };
                params.push(Param { label, ty });
            }
        }
        let returns = match root.get("returns") {
            Some(t) => TypeExpr::from_json(t)?,
            None => TypeExpr::Void,
        };
        let throws = matches!(root.get("throws"), Some(Json::Bool(true)));
        Ok(Signature {
            name,
            params,
            returns,
            throws,
        })
    }
}

/// Encode a runtime [`SwiftValue`] to JSON, validating it against `ty`.
///
/// A mismatch (wrong variant, non-string dictionary key, …) returns `Err` with
/// a human-readable reason; the trampoline turns that into a runtime type error
/// naming the function.
pub fn encode_value(value: &SwiftValue, ty: &TypeExpr) -> Result<Json, String> {
    match ty {
        TypeExpr::Void => match value {
            SwiftValue::Void => Ok(Json::Null),
            other => Err(format!("expected Void, got {}", other.type_name())),
        },
        TypeExpr::Bool => match value {
            SwiftValue::Bool(b) => Ok(Json::Bool(*b)),
            other => Err(format!("expected Bool, got {}", other.type_name())),
        },
        TypeExpr::Int => match value {
            SwiftValue::Int(i) => {
                // The wire `Int` is signed 64-bit. A wider/unsigned runtime
                // value (e.g. a `UInt` above `i64::MAX`) must not silently wrap
                // across the bridge — surface it as a runtime type error.
                let raw: i128 = i.raw;
                if raw < i64::MIN as i128 || raw > i64::MAX as i128 {
                    return Err(format!(
                        "expected Int, got {} value {} out of Int range",
                        i.width.type_name(),
                        raw
                    ));
                }
                Ok(Json::Int(raw as i64))
            }
            other => Err(format!("expected Int, got {}", other.type_name())),
        },
        TypeExpr::Double => match value {
            SwiftValue::Double(d) => Ok(Json::Double(*d)),
            // A literal like `3` in a Double context can arrive as an Int.
            SwiftValue::Int(i) => Ok(Json::Double(i.raw as f64)),
            other => Err(format!("expected Double, got {}", other.type_name())),
        },
        TypeExpr::String => match value {
            SwiftValue::Str(s) => Ok(Json::Str(s.clone())),
            SwiftValue::Substring { base, start, end } => {
                Ok(Json::Str(base[*start..*end].to_string()))
            }
            other => Err(format!("expected String, got {}", other.type_name())),
        },
        TypeExpr::Optional(inner) => match value {
            SwiftValue::Nil => Ok(Json::Null),
            other => encode_value(other, inner),
        },
        TypeExpr::Array(inner) => {
            let elems: &[SwiftValue] = match value {
                SwiftValue::Array(a) => a,
                SwiftValue::ArraySlice { base, start, end } => &base[*start..*end],
                other => return Err(format!("expected Array, got {}", other.type_name())),
            };
            let mut out = Vec::with_capacity(elems.len());
            for e in elems {
                out.push(encode_value(e, inner)?);
            }
            Ok(Json::Array(out))
        }
        TypeExpr::Dictionary(vty) => match value {
            SwiftValue::Dict(pairs) => {
                let mut out = Vec::with_capacity(pairs.len());
                for (k, v) in pairs.iter() {
                    let key = match k {
                        SwiftValue::Str(s) => s.clone(),
                        other => {
                            return Err(format!(
                                "dictionary key must be String, got {}",
                                other.type_name()
                            ))
                        }
                    };
                    out.push((key, encode_value(v, vty)?));
                }
                Ok(Json::Object(out))
            }
            other => Err(format!("expected Dictionary, got {}", other.type_name())),
        },
    }
}

/// Decode a JSON document to a runtime [`SwiftValue`], validating it against
/// `ty`. The inverse of [`encode_value`].
pub fn decode_value(node: &Json, ty: &TypeExpr) -> Result<SwiftValue, String> {
    match ty {
        TypeExpr::Void => Ok(SwiftValue::Void),
        TypeExpr::Bool => match node {
            Json::Bool(b) => Ok(SwiftValue::Bool(*b)),
            _ => Err("expected Bool".into()),
        },
        TypeExpr::Int => match node {
            Json::Int(i) => Ok(SwiftValue::Int(IntValue::int(*i as i128))),
            _ => Err("expected Int".into()),
        },
        TypeExpr::Double => match node {
            Json::Double(d) => Ok(SwiftValue::Double(*d)),
            Json::Int(i) => Ok(SwiftValue::Double(*i as f64)),
            _ => Err("expected Double".into()),
        },
        TypeExpr::String => match node {
            Json::Str(s) => Ok(SwiftValue::Str(s.clone())),
            _ => Err("expected String".into()),
        },
        TypeExpr::Optional(inner) => match node {
            Json::Null => Ok(SwiftValue::Nil),
            other => decode_value(other, inner),
        },
        TypeExpr::Array(inner) => match node {
            Json::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(decode_value(item, inner)?);
                }
                Ok(SwiftValue::Array(std::rc::Rc::new(out)))
            }
            _ => Err("expected Array".into()),
        },
        TypeExpr::Dictionary(vty) => match node {
            Json::Object(entries) => {
                let mut out = Vec::with_capacity(entries.len());
                for (k, v) in entries {
                    out.push((SwiftValue::Str(k.clone()), decode_value(v, vty)?));
                }
                Ok(SwiftValue::Dict(std::rc::Rc::new(out)))
            }
            _ => Err("expected Dictionary".into()),
        },
    }
}

/// A registered host function: its signature plus the handler that services it.
struct HostFn {
    signature: Signature,
    handler: Arc<dyn HostCallHandler>,
}

/// The interpreter-owned registry of host functions.
///
/// Owned by the [`Interpreter`][crate::Interpreter]; the trampoline consults it
/// by name. An optional default handler mirrors `set_http_transport`: functions
/// registered without an explicit handler fall back to it.
#[derive(Default)]
pub struct HostBridge {
    fns: HashMap<String, HostFn>,
    default_handler: Option<Arc<dyn HostCallHandler>>,
}

impl HostBridge {
    /// Install the default handler used by host functions registered without
    /// their own. Mirrors `Interpreter::set_http_transport`.
    pub fn set_handler(&mut self, handler: Arc<dyn HostCallHandler>) {
        self.default_handler = Some(handler);
    }

    /// Register a host function from its signature JSON, served by `handler`
    /// (or, if `None`, by the installed default handler at call time). Returns
    /// the registered function name.
    pub fn register(
        &mut self,
        signature_json: &str,
        handler: Option<Arc<dyn HostCallHandler>>,
    ) -> Result<String, String> {
        let signature = Signature::from_json(signature_json)?;
        let name = signature.name.clone();
        let handler = match handler.or_else(|| self.default_handler.clone()) {
            Some(h) => h,
            None => {
                return Err(format!(
                    "host fn `{name}` registered without a handler and no default handler is set"
                ))
            }
        };
        self.fns.insert(name.clone(), HostFn { signature, handler });
        Ok(name)
    }

    /// Whether `name` is a registered host function.
    pub fn contains(&self, name: &str) -> bool {
        self.fns.contains_key(name)
    }

    fn get(&self, name: &str) -> Option<&HostFn> {
        self.fns.get(name)
    }
}

/// The outcome of the host-call trampoline, before it is mapped onto the
/// interpreter's `Signal` channel.
#[derive(Debug)]
pub enum HostCallOutcome {
    /// A decoded, validated return value.
    Value(SwiftValue),
    /// The host raised a catchable Swift error. String payloads use the
    /// long-standing `HostError { message }` shape; structured payloads let a
    /// capability expose a portable, typed error without teaching core about
    /// any particular framework.
    Thrown {
        type_name: String,
        fields: Vec<(String, SwiftValue)>,
    },
}

impl HostBridge {
    /// Run the shared trampoline for a call to host function `name` with
    /// already-evaluated `(label, value)` arguments.
    ///
    /// Validates arity, labels, and argument types against the signature;
    /// encodes to a JSON array; invokes the handler; then decodes and validates
    /// the result against the return type. All failures are `Err(String)` with
    /// a message naming the function — the caller maps them onto a runtime type
    /// error. A `{"$thrown": …}` payload becomes [`HostCallOutcome::Thrown`].
    pub fn invoke(
        &self,
        name: &str,
        args: &[(Option<String>, SwiftValue)],
    ) -> Result<HostCallOutcome, String> {
        let host = self
            .get(name)
            .ok_or_else(|| format!("host fn `{name}` is not registered"))?;
        let sig = &host.signature;

        // Arity.
        if args.len() != sig.params.len() {
            return Err(format!(
                "host fn `{name}` expects {} argument(s), got {}",
                sig.params.len(),
                args.len()
            ));
        }

        // Labels + argument types, encoded in declared order.
        let mut encoded = Vec::with_capacity(args.len());
        for (i, (param, (label, value))) in sig.params.iter().zip(args).enumerate() {
            let expected = if param.label.is_empty() {
                None
            } else {
                Some(param.label.as_str())
            };
            if label.as_deref() != expected {
                return Err(format!(
                    "host fn `{name}` argument {i}: expected label {}, got {}",
                    fmt_label(expected),
                    fmt_label(label.as_deref())
                ));
            }
            let json = encode_value(value, &param.ty)
                .map_err(|e| format!("host fn `{name}` argument {i}: {e}"))?;
            encoded.push(json);
        }
        let args_json = json::to_string(&Json::Array(encoded));

        // Cross the boundary.
        let reply = host
            .handler
            .call(name, &args_json)
            .map_err(|e| format!("host fn `{name}` failed: {e}"))?;

        // Decode the reply.
        let root = json::parse(&reply)
            .map_err(|e| format!("host fn `{name}` returned invalid JSON: {e}"))?;

        // A `{"$thrown": <message>}` payload raises a catchable Swift error.
        // The dollar-prefixed key avoids colliding with a legitimate
        // dictionary result that happens to contain a `"thrown"` key.
        if let Some(thrown) = root.get("$thrown") {
            let (type_name, fields) = match thrown {
                Json::Str(message) => (
                    "HostError".to_string(),
                    vec![("message".to_string(), SwiftValue::Str(message.clone()))],
                ),
                Json::Object(values) => {
                    let type_name = match thrown.get("type") {
                        Some(Json::Str(name)) => name.clone(),
                        _ => "HostError".to_string(),
                    };
                    let fields = values
                        .iter()
                        .filter_map(|(name, value)| {
                            if name == "type" {
                                return None;
                            }
                            let value = match value {
                                Json::Str(value) => SwiftValue::Str(value.clone()),
                                Json::Int(value) => SwiftValue::int(i128::from(*value)),
                                Json::Bool(value) => SwiftValue::Bool(*value),
                                _ => return None,
                            };
                            Some((name.clone(), value))
                        })
                        .collect();
                    (type_name, fields)
                }
                other => (
                    "HostError".to_string(),
                    vec![(
                        "message".to_string(),
                        SwiftValue::Str(json::to_string(other)),
                    )],
                ),
            };
            return Ok(HostCallOutcome::Thrown { type_name, fields });
        }

        let value = decode_value(&root, &sig.returns)
            .map_err(|e| format!("host fn `{name}` returned a bad result: {e}"))?;
        Ok(HostCallOutcome::Value(value))
    }
}

fn fmt_label(label: Option<&str>) -> String {
    match label {
        Some(l) => format!("`{l}`"),
        None => "none".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scalar_and_compound_types() {
        assert_eq!(
            TypeExpr::from_json(&Json::Str("Int".into())).unwrap(),
            TypeExpr::Int
        );
        let opt = json::parse(r#"{"optional": "String"}"#).unwrap();
        assert_eq!(
            TypeExpr::from_json(&opt).unwrap(),
            TypeExpr::Optional(Box::new(TypeExpr::String))
        );
        let arr = json::parse(r#"{"array": {"optional": "Int"}}"#).unwrap();
        assert_eq!(
            TypeExpr::from_json(&arr).unwrap(),
            TypeExpr::Array(Box::new(TypeExpr::Optional(Box::new(TypeExpr::Int))))
        );
        let dict = json::parse(r#"{"dictionary": "Double"}"#).unwrap();
        assert_eq!(
            TypeExpr::from_json(&dict).unwrap(),
            TypeExpr::Dictionary(Box::new(TypeExpr::Double))
        );
    }

    #[test]
    fn rejects_unknown_type() {
        assert!(TypeExpr::from_json(&Json::Str("Banana".into())).is_err());
        let bad = json::parse(r#"{"tuple": "Int"}"#).unwrap();
        assert!(TypeExpr::from_json(&bad).is_err());
    }

    #[test]
    fn parses_full_signature() {
        let sig = Signature::from_json(
            r#"{"name":"greet","params":[{"label":"name","type":"String"}],"returns":"String","throws":true}"#,
        )
        .unwrap();
        assert_eq!(sig.name, "greet");
        assert_eq!(sig.params.len(), 1);
        assert_eq!(sig.params[0].label, "name");
        assert_eq!(sig.params[0].ty, TypeExpr::String);
        assert_eq!(sig.returns, TypeExpr::String);
        assert!(sig.throws);
    }

    #[test]
    fn signature_defaults_are_lenient() {
        let sig = Signature::from_json(r#"{"name":"ping"}"#).unwrap();
        assert!(sig.params.is_empty());
        assert_eq!(sig.returns, TypeExpr::Void);
        assert!(!sig.throws);
    }

    #[test]
    fn signature_requires_name() {
        assert!(Signature::from_json(r#"{"params":[]}"#).is_err());
    }

    #[test]
    fn encode_round_trips_through_decode() {
        let ty = TypeExpr::Array(Box::new(TypeExpr::Int));
        let value = SwiftValue::Array(std::rc::Rc::new(vec![
            SwiftValue::int(1),
            SwiftValue::int(2),
        ]));
        let json = encode_value(&value, &ty).unwrap();
        let back = decode_value(&json, &ty).unwrap();
        let SwiftValue::Array(items) = back else {
            panic!("expected array");
        };
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn encode_rejects_wrong_variant() {
        let err = encode_value(&SwiftValue::Bool(true), &TypeExpr::Int).unwrap_err();
        assert!(err.contains("expected Int"));
    }

    #[test]
    fn encode_rejects_uint_above_i64_max() {
        use crate::value::IntWidth;
        // A `UInt` whose value exceeds `i64::MAX` must not wrap silently.
        let value = SwiftValue::Int(IntValue::new(i64::MAX as i128 + 1, IntWidth::U64));
        let err = encode_value(&value, &TypeExpr::Int).unwrap_err();
        assert!(err.contains("out of Int range"), "{err}");
    }

    #[test]
    fn encode_accepts_i64_max() {
        use crate::value::IntWidth;
        let value = SwiftValue::Int(IntValue::new(i64::MAX as i128, IntWidth::I64));
        assert_eq!(
            encode_value(&value, &TypeExpr::Int).unwrap(),
            Json::Int(i64::MAX)
        );
    }

    #[test]
    fn encode_optional_nil_is_null() {
        let ty = TypeExpr::Optional(Box::new(TypeExpr::Int));
        assert_eq!(encode_value(&SwiftValue::Nil, &ty).unwrap(), Json::Null);
    }

    #[test]
    fn encode_string_dictionary() {
        let ty = TypeExpr::Dictionary(Box::new(TypeExpr::Int));
        let value = SwiftValue::Dict(std::rc::Rc::new(vec![(
            SwiftValue::Str("a".into()),
            SwiftValue::int(1),
        )]));
        let json = encode_value(&value, &ty).unwrap();
        assert_eq!(json.get("a"), Some(&Json::Int(1)));
    }

    // -- Trampoline ------------------------------------------------------

    struct EchoHandler;
    impl HostCallHandler for EchoHandler {
        fn call(&self, _name: &str, args_json: &str) -> Result<String, String> {
            // Signature: sum(_ a: Int, _ b: Int) -> Int
            let Json::Array(items) = json::parse(args_json).unwrap() else {
                return Err("expected array".into());
            };
            let (Json::Int(a), Json::Int(b)) = (&items[0], &items[1]) else {
                return Err("expected two ints".into());
            };
            Ok(format!("{}", a + b))
        }
    }

    fn sum_bridge() -> HostBridge {
        let mut bridge = HostBridge::default();
        bridge
            .register(
                r#"{"name":"sum","params":[{"type":"Int"},{"type":"Int"}],"returns":"Int"}"#,
                Some(Arc::new(EchoHandler)),
            )
            .unwrap();
        bridge
    }

    #[test]
    fn invoke_success() {
        let bridge = sum_bridge();
        let out = bridge
            .invoke(
                "sum",
                &[(None, SwiftValue::int(2)), (None, SwiftValue::int(3))],
            )
            .unwrap();
        match out {
            HostCallOutcome::Value(SwiftValue::Int(i)) => assert_eq!(i.raw, 5),
            _ => panic!("expected 5"),
        }
    }

    #[test]
    fn invoke_arity_mismatch() {
        let bridge = sum_bridge();
        let err = bridge
            .invoke("sum", &[(None, SwiftValue::int(2))])
            .unwrap_err();
        assert!(err.contains("expects 2 argument"), "{err}");
    }

    #[test]
    fn invoke_wrong_type() {
        let bridge = sum_bridge();
        let err = bridge
            .invoke(
                "sum",
                &[(None, SwiftValue::int(2)), (None, SwiftValue::Bool(true))],
            )
            .unwrap_err();
        assert!(err.contains("expected Int"), "{err}");
    }

    #[test]
    fn invoke_wrong_label() {
        let bridge = sum_bridge();
        let err = bridge
            .invoke(
                "sum",
                &[
                    (Some("x".into()), SwiftValue::int(2)),
                    (None, SwiftValue::int(3)),
                ],
            )
            .unwrap_err();
        assert!(err.contains("label"), "{err}");
    }

    struct BadResultHandler;
    impl HostCallHandler for BadResultHandler {
        fn call(&self, _name: &str, _args: &str) -> Result<String, String> {
            Ok(r#""not an int""#.into())
        }
    }

    #[test]
    fn invoke_bad_result() {
        let mut bridge = HostBridge::default();
        bridge
            .register(
                r#"{"name":"count","returns":"Int"}"#,
                Some(Arc::new(BadResultHandler)),
            )
            .unwrap();
        let err = bridge.invoke("count", &[]).unwrap_err();
        assert!(err.contains("bad result"), "{err}");
    }

    struct ThrowHandler;
    impl HostCallHandler for ThrowHandler {
        fn call(&self, _name: &str, _args: &str) -> Result<String, String> {
            Ok(r#"{"$thrown":"boom"}"#.into())
        }
    }

    #[test]
    fn invoke_thrown_payload() {
        let mut bridge = HostBridge::default();
        bridge
            .register(
                r#"{"name":"risky","returns":"Int","throws":true}"#,
                Some(Arc::new(ThrowHandler)),
            )
            .unwrap();
        match bridge.invoke("risky", &[]).unwrap() {
            HostCallOutcome::Thrown { type_name, fields } => {
                assert_eq!(type_name, "HostError");
                assert_eq!(
                    fields,
                    vec![("message".to_string(), SwiftValue::Str("boom".to_string()))]
                );
            }
            _ => panic!("expected thrown"),
        }
    }

    struct DictThrownKeyHandler;
    impl HostCallHandler for DictThrownKeyHandler {
        fn call(&self, _name: &str, _args: &str) -> Result<String, String> {
            // A legitimate `[String: String]` result that happens to carry a
            // `"thrown"` key must be returned as a value, not misread as an
            // error — the sentinel is `$thrown`, not `thrown`.
            Ok(r#"{"thrown":"value"}"#.into())
        }
    }

    #[test]
    fn invoke_dict_result_with_thrown_key_is_value() {
        let mut bridge = HostBridge::default();
        bridge
            .register(
                r#"{"name":"lookup","returns":{"dictionary":"String"}}"#,
                Some(Arc::new(DictThrownKeyHandler)),
            )
            .unwrap();
        match bridge.invoke("lookup", &[]).unwrap() {
            HostCallOutcome::Value(SwiftValue::Dict(pairs)) => {
                assert_eq!(pairs.len(), 1);
                assert!(matches!(&pairs[0].0, SwiftValue::Str(k) if k == "thrown"));
            }
            other => panic!("expected dict value, got {other:?}"),
        }
    }

    struct FailHandler;
    impl HostCallHandler for FailHandler {
        fn call(&self, _name: &str, _args: &str) -> Result<String, String> {
            Err("host exploded".into())
        }
    }

    #[test]
    fn invoke_handler_error() {
        let mut bridge = HostBridge::default();
        bridge
            .register(
                r#"{"name":"boom","returns":"Void"}"#,
                Some(Arc::new(FailHandler)),
            )
            .unwrap();
        let err = bridge.invoke("boom", &[]).unwrap_err();
        assert!(
            err.contains("boom") && err.contains("host exploded"),
            "{err}"
        );
    }

    #[test]
    fn default_handler_used_when_none_supplied() {
        let mut bridge = HostBridge::default();
        bridge.set_handler(Arc::new(EchoHandler));
        bridge
            .register(
                r#"{"name":"sum","params":[{"type":"Int"},{"type":"Int"}],"returns":"Int"}"#,
                None,
            )
            .unwrap();
        assert!(bridge.contains("sum"));
    }

    #[test]
    fn register_without_handler_fails() {
        let mut bridge = HostBridge::default();
        assert!(bridge
            .register(r#"{"name":"sum","returns":"Void"}"#, None)
            .is_err());
    }
}
