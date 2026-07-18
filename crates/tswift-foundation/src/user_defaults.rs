//! `UserDefaults` — Foundation's key-value defaults store, backed by the
//! `tswift.defaults` host service ([`tswift_core::host_services`]).
//!
//! ## Host wire schema
//!
//! Three host functions, all declared with stage-1 `String` signatures (see
//! `tswift_core::host_bridge`). Heterogeneous storage travels as a JSON
//! document ([`tswift_core::json`]) so one wire shape covers every stored
//! Swift type without widening the stage-1 type vocabulary:
//!
//! - `tswift.defaults.set(key: String, value: String) -> Void` — `value` is
//!   the JSON encoding of the stored value (`true`, `42`, `"hi"`,
//!   `["a","b"]`).
//! - `tswift.defaults.get(key: String) -> String?` — the JSON encoding of the
//!   stored value, or `nil` if the key is absent.
//! - `tswift.defaults.remove(key: String) -> Void`.
//!
//! Foundation (this module) only *declares* these signatures via
//! [`tswift_core::Interpreter::register_host_fn`] when the platform's
//! [`Capabilities`] backs [`HostService::Defaults`]; the platform embedding
//! supplies the actual handler via `Interpreter::set_host_call_handler` (or a
//! per-function handler). What backs the store — in-memory, a file,
//! `localStorage`, real `UserDefaults`, … — is entirely the host's business.
//!
//! ## Type coercion
//!
//! Foundation's typed accessors coerce a stored value of another type rather
//! than failing (`NSNumber`/`NSString` bridging). This runtime approximates
//! the documented behaviour:
//!
//! - `bool(forKey:)`: `Bool` as-is; `Int`/`Double` → `true` iff non-zero;
//!   `String` → `NSString.boolValue` semantics (`true` iff, after skipping
//!   nothing, the first character is `Y`/`y`/`T`/`t` or a digit `1`-`9`);
//!   missing key or any other stored type → `false`.
//! - `integer(forKey:)`: `Int` as-is; `Double` truncated toward zero;
//!   `Bool` → `1`/`0`; `String` → `NSString.integerValue` semantics (an
//!   optional leading sign followed by digits; `0` if the string has no
//!   numeric prefix); missing key or any other stored type → `0`.
//! - `double(forKey:)`: `Double` as-is; `Int` widened; `Bool` → `1.0`/`0.0`;
//!   `String` → its leading numeric prefix, `0.0` if none; missing key or any
//!   other stored type → `0.0`.
//! - `string(forKey:)`: `Some` only when the stored value is itself a
//!   `String` — Foundation does not stringify numbers/bools here; missing key
//!   or any other stored type → `nil`.
//! - `array(forKey:)` / `stringArray(forKey:)`: `Some` only when the stored
//!   value is an array (this runtime's `UserDefaults` only ever stores
//!   `[String]` — see Deviations below, so the two accessors coincide);
//!   missing key or any other stored type → `nil`.
//! - `object(forKey:)`: the stored value using its natural Swift type
//!   (`Bool`/`Int`/`Double`/`String`/`[String]`), or `nil` if the key is
//!   absent.
//!
//! ## Deviations from real Foundation
//!
//! - Only `Bool`, `Int`, `Double`, `String`, and `[String]` are storable —
//!   `set(_:forKey:)` traps on any other value shape. Real `UserDefaults`
//!   accepts any property-list-representable value (nested arrays/
//!   dictionaries, `Data`, `Date`); widening the stored-value vocabulary is
//!   future work, not a deliberate semantic choice.
//! - `dictionaryRepresentation()` is not implemented: no `tswift.defaults`
//!   host function enumerates keys. Deferred rather than faked with a partial
//!   result.
//! - `register(defaults:)`, `synchronize()`, KVO/`NotificationCenter`
//!   integration, and suites (`UserDefaults(suiteName:)`) are not
//!   implemented.

use std::cell::RefCell;
use std::rc::Rc;

use tswift_core::json::{self, Json};
use tswift_core::{
    Arg, BuiltinReceiver, ClassObj, HostService, Interpreter, MethodEntry, Outcome, StdContext,
    SwiftValue,
};

use crate::{data_bytes, data_value, type_error};

/// The stable [`BuiltinReceiver`] key for `UserDefaults`, minted once per
/// process via [`BuiltinReceiver::register_extension`]. Core owns no
/// knowledge of the `"UserDefaults"` name — it is this crate's opaque string,
/// handed to core's generic extension-registration seam so `UserDefaults`
/// gets the same `(BuiltinReceiver, method-name)` dispatch tables as any
/// built-in receiver, without core's `BuiltinReceiver` enum ever spelling a
/// Foundation type name.
fn receiver() -> BuiltinReceiver {
    BuiltinReceiver::register_extension("UserDefaults")
}

/// The [`StdContext::singleton`] cache key for `UserDefaults.standard`.
const STANDARD_KEY: &str = "UserDefaults.standard";

/// Register `UserDefaults` into `interp`. When `available` is `false` (the
/// `tswift.defaults` host service is not backed by the current platform),
/// `UserDefaults.standard` and its methods are still registered, but every
/// method body raises the capability diagnostic instead of touching the host.
pub(crate) fn install(interp: &mut Interpreter<'_>, available: bool) {
    if available {
        // Declare the three host-function signatures; the platform supplies
        // the handler via `Interpreter::set_host_call_handler` (or its own
        // per-function handler passed to `register_host_fn` directly, which
        // this crate cannot do — Foundation only owns the wire contract).
        //
        // Registration fails (returns `Err`) only when the embedding declared
        // `HostService::Defaults` available but never installed a call
        // handler. Rather than panic an install that already promised to be
        // behaviour-preserving, that degrades gracefully: `is_host_fn` then
        // reports `false` and every `UserDefaults` method raises the same
        // "unavailable on this platform" diagnostic as a platform that never
        // declared the service, instead of a runtime host-probe surprise.
        let _ = interp.register_host_fn(
            r#"{"name":"tswift.defaults.set","params":[{"label":"key","type":"String"},{"label":"value","type":"String"}],"returns":"Void"}"#,
            None,
        );
        let _ = interp.register_host_fn(
            r#"{"name":"tswift.defaults.get","params":[{"label":"key","type":"String"}],"returns":{"optional":"String"}}"#,
            None,
        );
        let _ = interp.register_host_fn(
            r#"{"name":"tswift.defaults.remove","params":[{"label":"key","type":"String"}],"returns":"Void"}"#,
            None,
        );
        let _ = interp.register_host_fn(
            r#"{"name":"tswift.defaults.register","params":[{"label":"defaults","type":"String"}],"returns":"Void"}"#,
            None,
        );
    }

    // Resolved via `register_static` (a `StaticFn`, checked first for the
    // qualified `UserDefaults.standard` form — see `urlsession.rs`'s
    // `URLSessionConfiguration.default` for the same pattern), never via
    // `register_static_value`. The latter inserts into the interpreter's
    // single global `statics` map, which *also* backs the ambiguous bare
    // `.name` shorthand fallback (`resolve_implicit_static`): any other
    // builtin's `.standard` (e.g. `Date.FormatStyle.TimeStyle.standard`)
    // accessed without a resolvable contextual type would silently pick up
    // this entry instead once it became the map's *unique* `.standard`
    // suffix match — not an ambiguity error, a wrong-value bug. Identity is
    // still real, though: `ud_standard_static` fetches the singleton
    // `Object` through `StdContext::singleton` (a per-interpreter cache
    // keyed only by the exact `"UserDefaults.standard"` string, never
    // consulted by that bare-shorthand fallback), so `UserDefaults.standard
    // === UserDefaults.standard` holds, matching real Foundation.
    interp.register_static(receiver(), "standard", ud_standard_static);

    interp.register_intrinsic(
        receiver(),
        "set",
        MethodEntry {
            mutating: false,
            func: ud_set,
        },
    );
    interp.register_intrinsic(
        receiver(),
        "bool",
        MethodEntry {
            mutating: false,
            func: ud_bool,
        },
    );
    interp.register_intrinsic(
        receiver(),
        "integer",
        MethodEntry {
            mutating: false,
            func: ud_integer,
        },
    );
    interp.register_intrinsic(
        receiver(),
        "double",
        MethodEntry {
            mutating: false,
            func: ud_double,
        },
    );
    interp.register_intrinsic(
        receiver(),
        "string",
        MethodEntry {
            mutating: false,
            func: ud_string,
        },
    );
    interp.register_intrinsic(
        receiver(),
        "array",
        MethodEntry {
            mutating: false,
            func: ud_string_array,
        },
    );
    interp.register_intrinsic(
        receiver(),
        "stringArray",
        MethodEntry {
            mutating: false,
            func: ud_string_array,
        },
    );
    interp.register_intrinsic(
        receiver(),
        "dictionary",
        MethodEntry {
            mutating: false,
            func: ud_dictionary,
        },
    );
    interp.register_intrinsic(
        receiver(),
        "data",
        MethodEntry {
            mutating: false,
            func: ud_data,
        },
    );
    interp.register_intrinsic(
        receiver(),
        "register",
        MethodEntry {
            mutating: false,
            func: ud_register,
        },
    );
    interp.register_intrinsic(
        receiver(),
        "object",
        MethodEntry {
            mutating: false,
            func: ud_object,
        },
    );
    interp.register_intrinsic(
        receiver(),
        "removeObject",
        MethodEntry {
            mutating: false,
            func: ud_remove_object,
        },
    );
}

fn ud_standard_static(ctx: &mut dyn StdContext, args: Vec<Arg>) -> tswift_core::StdResult {
    if !args.is_empty() {
        return Err(type_error("UserDefaults.standard expects no arguments"));
    }
    Ok(ctx.singleton(STANDARD_KEY, user_defaults_value))
}

fn user_defaults_value() -> SwiftValue {
    SwiftValue::Object(Rc::new(RefCell::new(ClassObj {
        class_name: "UserDefaults".into(),
        fields: Vec::new(),
    })))
}

/// The capability-gated diagnostic raised by every `UserDefaults` method when
/// the host does not back [`HostService::Defaults`]. Constructed by hand
/// (rather than via `Capabilities::require`) because availability is only
/// knowable at call time through [`StdContext::is_host_fn`] — the `caps`
/// value itself does not cross into a `fn`-pointer method body.
fn unavailable() -> tswift_core::StdError {
    tswift_core::StdError::Error(tswift_core::EvalError::Type(
        tswift_core::CapabilityError {
            service: HostService::Defaults,
            api: "UserDefaults".to_string(),
        }
        .to_string(),
    ))
}

fn require_key(args: &[SwiftValue], who: &str) -> Result<String, tswift_core::StdError> {
    match args.first() {
        Some(SwiftValue::Str(s)) => Ok(s.clone()),
        Some(SwiftValue::Substring { base, start, end }) => Ok(base[*start..*end].to_string()),
        _ => Err(type_error(format!("{who} expects a String key"))),
    }
}

// ---------------------------------------------------------------------------
// set(_:forKey:)
// ---------------------------------------------------------------------------

fn ud_set(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, tswift_core::StdError> {
    if !ctx.is_host_fn("tswift.defaults.set") {
        return Err(unavailable());
    }
    if args.len() != 2 {
        return Err(type_error("set(_:forKey:) expects two arguments"));
    }
    let key = match &args[1] {
        SwiftValue::Str(s) => s.clone(),
        SwiftValue::Substring { base, start, end } => base[*start..*end].to_string(),
        _ => return Err(type_error("set(_:forKey:) expects a String key")),
    };
    // `set(nil, forKey:)` removes the object, matching Foundation.
    if matches!(&args[0], SwiftValue::Nil) {
        return ud_remove(ctx, recv, key);
    }
    let encoded = encode_stored_value(&args[0])?;
    let value_json = json::to_string(&encoded);
    ctx.call_host_fn(
        "tswift.defaults.set",
        vec![
            (Some("key".to_string()), SwiftValue::Str(key)),
            (Some("value".to_string()), SwiftValue::Str(value_json)),
        ],
    )?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: recv,
    })
}

fn ud_remove(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    key: String,
) -> Result<Outcome, tswift_core::StdError> {
    ctx.call_host_fn(
        "tswift.defaults.remove",
        vec![(Some("key".to_string()), SwiftValue::Str(key))],
    )?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: recv,
    })
}

fn ud_remove_object(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, tswift_core::StdError> {
    if !ctx.is_host_fn("tswift.defaults.remove") {
        return Err(unavailable());
    }
    let key = require_key(&args, "removeObject(forKey:)")?;
    ud_remove(ctx, recv, key)
}

/// Encode a value accepted by `set(_:forKey:)` to its stored JSON
/// representation. See the module docs' Deviations section for the supported
/// shapes.
fn encode_stored_value(value: &SwiftValue) -> Result<Json, tswift_core::StdError> {
    match value {
        SwiftValue::Bool(b) => Ok(Json::Bool(*b)),
        SwiftValue::Int(i) => i.raw.try_into().map(Json::Int).map_err(|_| {
            type_error(format!(
                "arithmetic overflow storing {} in UserDefaults: it does not fit in Int64",
                i.raw
            ))
        }),
        SwiftValue::Double(d) => Ok(Json::Double(*d)),
        SwiftValue::Str(s) => Ok(Json::Str(s.clone())),
        SwiftValue::Substring { base, start, end } => Ok(Json::Str(base[*start..*end].to_string())),
        SwiftValue::Struct(obj) if obj.type_name == "Data" => Ok(Json::Object(vec![(
            "$tswiftData".to_string(),
            Json::Str(tswift_core::base64::encode(&data_bytes(value)?)),
        )])),
        SwiftValue::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items.iter() {
                out.push(encode_stored_value(item)?);
            }
            Ok(Json::Array(out))
        }
        SwiftValue::Dict(pairs) => {
            let mut out = Vec::with_capacity(pairs.len());
            for (key, value) in pairs.iter() {
                let key = match key {
                    SwiftValue::Str(key) => key.clone(),
                    other => {
                        return Err(type_error(format!(
                            "UserDefaults.set(_:forKey:) requires String dictionary keys, got {}",
                            other.type_name()
                        )))
                    }
                };
                out.push((key, encode_stored_value(value)?));
            }
            Ok(Json::Object(out))
        }
        other => Err(type_error(format!(
            "UserDefaults.set(_:forKey:) does not support storing {}",
            other.type_name()
        ))),
    }
}

// ---------------------------------------------------------------------------
// Typed getters
// ---------------------------------------------------------------------------

/// Fetch the raw stored JSON for `key`, or `None` if absent.
fn fetch(ctx: &mut dyn StdContext, key: String) -> Result<Option<Json>, tswift_core::StdError> {
    let result = ctx.call_host_fn(
        "tswift.defaults.get",
        vec![(Some("key".to_string()), SwiftValue::Str(key))],
    )?;
    match result {
        SwiftValue::Nil => Ok(None),
        SwiftValue::Str(s) => json::parse(&s)
            .map(Some)
            .map_err(|e| type_error(format!("UserDefaults: host returned invalid JSON: {e}"))),
        other => Err(type_error(format!(
            "UserDefaults: host `tswift.defaults.get` returned {}, expected String?",
            other.type_name()
        ))),
    }
}

fn ud_bool(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, tswift_core::StdError> {
    if !ctx.is_host_fn("tswift.defaults.get") {
        return Err(unavailable());
    }
    let key = require_key(&args, "bool(forKey:)")?;
    let stored = fetch(ctx, key)?;
    let result = SwiftValue::Bool(match stored {
        Some(Json::Bool(b)) => b,
        Some(Json::Int(i)) => i != 0,
        Some(Json::Double(d)) => d != 0.0,
        Some(Json::Str(s)) => ns_string_bool_value(&s),
        _ => false,
    });
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

fn ud_integer(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, tswift_core::StdError> {
    if !ctx.is_host_fn("tswift.defaults.get") {
        return Err(unavailable());
    }
    let key = require_key(&args, "integer(forKey:)")?;
    let stored = fetch(ctx, key)?;
    let n = match stored {
        Some(Json::Int(i)) => i,
        Some(Json::Double(d)) => d.trunc() as i64,
        Some(Json::Bool(b)) => i64::from(b),
        Some(Json::Str(s)) => ns_string_integer_value(&s),
        _ => 0,
    };
    Ok(Outcome {
        result: SwiftValue::int(n as i128),
        receiver: recv,
    })
}

fn ud_double(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, tswift_core::StdError> {
    if !ctx.is_host_fn("tswift.defaults.get") {
        return Err(unavailable());
    }
    let key = require_key(&args, "double(forKey:)")?;
    let stored = fetch(ctx, key)?;
    let d = match stored {
        Some(Json::Double(d)) => d,
        Some(Json::Int(i)) => i as f64,
        Some(Json::Bool(b)) => {
            if b {
                1.0
            } else {
                0.0
            }
        }
        Some(Json::Str(s)) => ns_string_double_value(&s),
        _ => 0.0,
    };
    Ok(Outcome {
        result: SwiftValue::Double(d),
        receiver: recv,
    })
}

fn ud_string(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, tswift_core::StdError> {
    if !ctx.is_host_fn("tswift.defaults.get") {
        return Err(unavailable());
    }
    let key = require_key(&args, "string(forKey:)")?;
    let stored = fetch(ctx, key)?;
    let result = match stored {
        Some(Json::Str(s)) => SwiftValue::Str(s),
        _ => SwiftValue::Nil,
    };
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

fn ud_string_array(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, tswift_core::StdError> {
    if !ctx.is_host_fn("tswift.defaults.get") {
        return Err(unavailable());
    }
    let key = require_key(&args, "stringArray(forKey:)")?;
    let stored = fetch(ctx, key)?;
    let result = match stored {
        Some(Json::Array(items)) => {
            let mut strs = Vec::with_capacity(items.len());
            let mut all_strings = true;
            for item in items {
                match item {
                    Json::Str(s) => strs.push(SwiftValue::Str(s)),
                    _ => {
                        all_strings = false;
                        break;
                    }
                }
            }
            if all_strings {
                SwiftValue::Array(Rc::new(strs))
            } else {
                SwiftValue::Nil
            }
        }
        _ => SwiftValue::Nil,
    };
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

fn ud_object(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, tswift_core::StdError> {
    if !ctx.is_host_fn("tswift.defaults.get") {
        return Err(unavailable());
    }
    let key = require_key(&args, "object(forKey:)")?;
    let stored = fetch(ctx, key)?;
    let result = match stored {
        Some(json) => decode_stored_value(&json),
        None => SwiftValue::Nil,
    };
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

fn ud_dictionary(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, tswift_core::StdError> {
    if !ctx.is_host_fn("tswift.defaults.get") {
        return Err(unavailable());
    }
    let key = require_key(&args, "dictionary(forKey:)")?;
    let result = match fetch(ctx, key)? {
        Some(Json::Object(values)) if values.iter().all(|(key, _)| key != "$tswiftData") => {
            SwiftValue::Dict(Rc::new(
                values
                    .iter()
                    .map(|(key, value)| (SwiftValue::Str(key.clone()), decode_stored_value(value)))
                    .collect(),
            ))
        }
        _ => SwiftValue::Nil,
    };
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

fn ud_data(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, tswift_core::StdError> {
    if !ctx.is_host_fn("tswift.defaults.get") {
        return Err(unavailable());
    }
    let key = require_key(&args, "data(forKey:)")?;
    let result = match fetch(ctx, key)? {
        Some(Json::Object(values)) => values
            .iter()
            .find(|(key, _)| key == "$tswiftData")
            .and_then(|(_, value)| match value {
                Json::Str(value) => tswift_core::base64::decode(value).map(data_value),
                _ => None,
            })
            .unwrap_or(SwiftValue::Nil),
        _ => SwiftValue::Nil,
    };
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

fn ud_register(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, tswift_core::StdError> {
    if !ctx.is_host_fn("tswift.defaults.register") {
        return Err(unavailable());
    }
    let Some(SwiftValue::Dict(values)) = args.first() else {
        return Err(type_error("register(defaults:) expects a Dictionary"));
    };
    let mut encoded = Vec::with_capacity(values.len());
    for (key, value) in values.iter() {
        let SwiftValue::Str(key) = key else {
            return Err(type_error(
                "register(defaults:) requires String dictionary keys",
            ));
        };
        encoded.push((key.clone(), encode_stored_value(value)?));
    }
    ctx.call_host_fn(
        "tswift.defaults.register",
        vec![(
            Some("defaults".to_string()),
            SwiftValue::Str(json::to_string(&Json::Object(encoded))),
        )],
    )?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: recv,
    })
}

fn decode_stored_value(json: &Json) -> SwiftValue {
    match json {
        Json::Null => SwiftValue::Nil,
        Json::Bool(b) => SwiftValue::Bool(*b),
        Json::Int(i) => SwiftValue::int(*i as i128),
        Json::Double(d) => SwiftValue::Double(*d),
        Json::Str(s) => SwiftValue::Str(s.clone()),
        Json::Array(items) => {
            SwiftValue::Array(Rc::new(items.iter().map(decode_stored_value).collect()))
        }
        Json::Object(values) => {
            if let Some(Json::Str(bytes)) = values
                .iter()
                .find_map(|(key, value)| (key == "$tswiftData").then_some(value))
            {
                return tswift_core::base64::decode(bytes)
                    .map(data_value)
                    .unwrap_or(SwiftValue::Nil);
            }
            SwiftValue::Dict(Rc::new(
                values
                    .iter()
                    .map(|(key, value)| (SwiftValue::Str(key.clone()), decode_stored_value(value)))
                    .collect(),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// NSString bridging coercions (approximated per Apple's documented behaviour)
// ---------------------------------------------------------------------------

/// `NSString.boolValue`: `true` iff the string, after skipping leading
/// whitespace, starts with `Y`/`y`/`T`/`t` or a digit `1`-`9`.
fn ns_string_bool_value(s: &str) -> bool {
    match s.trim_start().chars().next() {
        Some(c) => matches!(c, 'Y' | 'y' | 'T' | 't' | '1'..='9'),
        None => false,
    }
}

/// `NSString.integerValue`: an optional leading `+`/`-` followed by decimal
/// digits, parsed from the start of the (whitespace-trimmed) string; `0` if
/// there is no numeric prefix.
fn ns_string_integer_value(s: &str) -> i64 {
    let trimmed = s.trim_start();
    let mut end = 0;
    let bytes = trimmed.as_bytes();
    if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
        end += 1;
    }
    let digits_start = end;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == digits_start {
        return 0;
    }
    trimmed[..end].parse::<i64>().unwrap_or(0)
}

/// `NSString.doubleValue`: the leading numeric prefix (optional sign, digits,
/// optional `.digits`, optional exponent), `0.0` if there is none.
fn ns_string_double_value(s: &str) -> f64 {
    let trimmed = s.trim_start();
    let bytes = trimmed.as_bytes();
    let mut end = 0;
    if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
        end += 1;
    }
    let mut saw_digit = false;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
        saw_digit = true;
    }
    if end < bytes.len() && bytes[end] == b'.' {
        let mut frac_end = end + 1;
        while frac_end < bytes.len() && bytes[frac_end].is_ascii_digit() {
            frac_end += 1;
            saw_digit = true;
        }
        // `frac_end` only grows when a fractional digit was actually
        // consumed above, or stays at `end + 1` (just past the `.`) with no
        // digits after it; either way, once any digit (integer or fraction)
        // has been seen, the `.` and its digits are part of the number.
        if saw_digit {
            end = frac_end;
        }
    }
    if !saw_digit {
        return 0.0;
    }
    if end < bytes.len() && (bytes[end] == b'e' || bytes[end] == b'E') {
        let mut exp_end = end + 1;
        if exp_end < bytes.len() && (bytes[exp_end] == b'+' || bytes[exp_end] == b'-') {
            exp_end += 1;
        }
        let exp_digits_start = exp_end;
        while exp_end < bytes.len() && bytes[exp_end].is_ascii_digit() {
            exp_end += 1;
        }
        if exp_end > exp_digits_start {
            end = exp_end;
        }
    }
    trimmed[..end].parse::<f64>().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use tswift_core::HostCallHandler;

    /// A real [`HostCallHandler`] backing `tswift.defaults.*` with an
    /// in-memory map, installed through [`Interpreter::set_host_call_handler`]
    /// exactly as a platform embedding (CLI/wasm/iOS) would — so tests drive
    /// the *full* wire (validate → encode → invoke → decode through
    /// [`tswift_core::host_bridge`]), not a hand-rolled `StdContext` shortcut.
    struct DefaultsHandler {
        store: Mutex<HashMap<String, String>>,
    }

    impl DefaultsHandler {
        fn new() -> Self {
            Self {
                store: Mutex::new(HashMap::new()),
            }
        }
    }

    impl HostCallHandler for DefaultsHandler {
        fn call(&self, name: &str, args_json: &str) -> Result<String, String> {
            let Json::Array(items) = json::parse(args_json).map_err(|e| e.to_string())? else {
                return Err("expected a JSON array of arguments".into());
            };
            let str_arg = |i: usize| -> Result<String, String> {
                match items.get(i) {
                    Some(Json::Str(s)) => Ok(s.clone()),
                    _ => Err(format!("{name}: expected a String argument at index {i}")),
                }
            };
            let mut store = self.store.lock().unwrap();
            match name {
                "tswift.defaults.set" => {
                    store.insert(str_arg(0)?, str_arg(1)?);
                    Ok("null".to_string())
                }
                "tswift.defaults.get" => match store.get(&str_arg(0)?) {
                    // The declared return type is `String?`; the reply is the
                    // JSON encoding of that optional String, i.e. the stored
                    // (already-JSON-encoded) text re-encoded as a JSON string
                    // literal — double-encoded, per the module docs' host
                    // wire schema.
                    Some(v) => Ok(json::to_string(&Json::Str(v.clone()))),
                    None => Ok("null".to_string()),
                },
                "tswift.defaults.remove" => {
                    store.remove(&str_arg(0)?);
                    Ok("null".to_string())
                }
                other => Err(format!("unexpected host fn {other}")),
            }
        }
    }

    /// Build an `Interpreter` with `UserDefaults` installed and, when
    /// `available`, a real [`DefaultsHandler`] wired through
    /// `set_host_call_handler`, then hand it to `f`. `Interpreter<'_>`
    /// borrows its output sink, so the interpreter can't outlive this
    /// function — callers get their result back through `f`'s return value.
    fn with_interp<R>(available: bool, f: impl FnOnce(&mut Interpreter) -> R) -> R {
        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        if available {
            interp.set_host_call_handler(Arc::new(DefaultsHandler::new()));
        }
        install(&mut interp, available);
        f(&mut interp)
    }

    fn standard() -> SwiftValue {
        user_defaults_value()
    }

    fn call(
        ctx: &mut dyn StdContext,
        method: fn(
            &mut dyn StdContext,
            SwiftValue,
            Vec<SwiftValue>,
        ) -> Result<Outcome, tswift_core::StdError>,
        args: Vec<SwiftValue>,
    ) -> SwiftValue {
        method(ctx, standard(), args).unwrap().result
    }

    #[test]
    fn set_and_get_bool_round_trips() {
        with_interp(true, |interp| {
            call(
                interp,
                ud_set,
                vec![SwiftValue::Bool(true), SwiftValue::Str("flag".into())],
            );
            assert_eq!(
                call(interp, ud_bool, vec![SwiftValue::Str("flag".into())]),
                SwiftValue::Bool(true)
            );
        });
    }

    #[test]
    fn set_and_get_int_round_trips() {
        with_interp(true, |interp| {
            call(
                interp,
                ud_set,
                vec![SwiftValue::int(42), SwiftValue::Str("count".into())],
            );
            let SwiftValue::Int(i) =
                call(interp, ud_integer, vec![SwiftValue::Str("count".into())])
            else {
                panic!("expected Int");
            };
            assert_eq!(i.raw, 42);
        });
    }

    #[test]
    fn set_and_get_double_round_trips() {
        with_interp(true, |interp| {
            call(
                interp,
                ud_set,
                vec![SwiftValue::Double(3.5), SwiftValue::Str("ratio".into())],
            );
            assert_eq!(
                call(interp, ud_double, vec![SwiftValue::Str("ratio".into())]),
                SwiftValue::Double(3.5)
            );
        });
    }

    #[test]
    fn set_and_get_string_round_trips() {
        with_interp(true, |interp| {
            call(
                interp,
                ud_set,
                vec![SwiftValue::Str("hi".into()), SwiftValue::Str("name".into())],
            );
            assert_eq!(
                call(interp, ud_string, vec![SwiftValue::Str("name".into())]),
                SwiftValue::Str("hi".into())
            );
        });
    }

    #[test]
    fn set_and_get_string_array_round_trips() {
        with_interp(true, |interp| {
            let arr = SwiftValue::Array(Rc::new(vec![
                SwiftValue::Str("a".into()),
                SwiftValue::Str("b".into()),
            ]));
            call(interp, ud_set, vec![arr, SwiftValue::Str("tags".into())]);
            let SwiftValue::Array(items) = call(
                interp,
                ud_string_array,
                vec![SwiftValue::Str("tags".into())],
            ) else {
                panic!("expected Array");
            };
            assert_eq!(items.len(), 2);
        });
    }

    #[test]
    fn missing_key_returns_documented_defaults() {
        with_interp(true, |interp| {
            let key = || SwiftValue::Str("missing".into());
            assert_eq!(call(interp, ud_bool, vec![key()]), SwiftValue::Bool(false));
            let SwiftValue::Int(i) = call(interp, ud_integer, vec![key()]) else {
                panic!("expected Int");
            };
            assert_eq!(i.raw, 0);
            assert_eq!(
                call(interp, ud_double, vec![key()]),
                SwiftValue::Double(0.0)
            );
            assert_eq!(call(interp, ud_string, vec![key()]), SwiftValue::Nil);
            assert_eq!(call(interp, ud_string_array, vec![key()]), SwiftValue::Nil);
            assert_eq!(call(interp, ud_object, vec![key()]), SwiftValue::Nil);
        });
    }

    #[test]
    fn bool_coerces_stored_int_and_string() {
        with_interp(true, |interp| {
            call(
                interp,
                ud_set,
                vec![SwiftValue::int(0), SwiftValue::Str("zero".into())],
            );
            call(
                interp,
                ud_set,
                vec![SwiftValue::int(7), SwiftValue::Str("seven".into())],
            );
            call(
                interp,
                ud_set,
                vec![SwiftValue::Str("YES".into()), SwiftValue::Str("yes".into())],
            );
            call(
                interp,
                ud_set,
                vec![SwiftValue::Str("no".into()), SwiftValue::Str("no".into())],
            );
            assert_eq!(
                call(interp, ud_bool, vec![SwiftValue::Str("zero".into())]),
                SwiftValue::Bool(false)
            );
            assert_eq!(
                call(interp, ud_bool, vec![SwiftValue::Str("seven".into())]),
                SwiftValue::Bool(true)
            );
            assert_eq!(
                call(interp, ud_bool, vec![SwiftValue::Str("yes".into())]),
                SwiftValue::Bool(true)
            );
            assert_eq!(
                call(interp, ud_bool, vec![SwiftValue::Str("no".into())]),
                SwiftValue::Bool(false)
            );
        });
    }

    #[test]
    fn integer_coerces_stored_double_and_string() {
        with_interp(true, |interp| {
            call(
                interp,
                ud_set,
                vec![SwiftValue::Double(9.9), SwiftValue::Str("pi".into())],
            );
            call(
                interp,
                ud_set,
                vec![
                    SwiftValue::Str("12abc".into()),
                    SwiftValue::Str("num".into()),
                ],
            );
            call(
                interp,
                ud_set,
                vec![
                    SwiftValue::Str("abc".into()),
                    SwiftValue::Str("notnum".into()),
                ],
            );
            let SwiftValue::Int(pi) = call(interp, ud_integer, vec![SwiftValue::Str("pi".into())])
            else {
                panic!("expected Int");
            };
            assert_eq!(pi.raw, 9);
            let SwiftValue::Int(num) =
                call(interp, ud_integer, vec![SwiftValue::Str("num".into())])
            else {
                panic!("expected Int");
            };
            assert_eq!(num.raw, 12);
            let SwiftValue::Int(notnum) =
                call(interp, ud_integer, vec![SwiftValue::Str("notnum".into())])
            else {
                panic!("expected Int");
            };
            assert_eq!(notnum.raw, 0);
        });
    }

    #[test]
    fn double_coerces_stored_string_prefix() {
        assert_eq!(ns_string_double_value("3.75xyz"), 3.75);
        assert_eq!(ns_string_double_value("not a number"), 0.0);
        assert_eq!(ns_string_double_value("  -2.5"), -2.5);
    }

    #[test]
    fn remove_object_deletes_key() {
        with_interp(true, |interp| {
            call(
                interp,
                ud_set,
                vec![SwiftValue::Str("x".into()), SwiftValue::Str("k".into())],
            );
            call(interp, ud_remove_object, vec![SwiftValue::Str("k".into())]);
            assert_eq!(
                call(interp, ud_string, vec![SwiftValue::Str("k".into())]),
                SwiftValue::Nil
            );
        });
    }

    #[test]
    fn set_nil_removes_object() {
        with_interp(true, |interp| {
            call(
                interp,
                ud_set,
                vec![SwiftValue::Str("x".into()), SwiftValue::Str("k".into())],
            );
            call(
                interp,
                ud_set,
                vec![SwiftValue::Nil, SwiftValue::Str("k".into())],
            );
            assert_eq!(
                call(interp, ud_string, vec![SwiftValue::Str("k".into())]),
                SwiftValue::Nil
            );
        });
    }

    #[test]
    fn object_for_key_returns_typed_any() {
        with_interp(true, |interp| {
            call(
                interp,
                ud_set,
                vec![SwiftValue::int(42), SwiftValue::Str("n".into())],
            );
            let SwiftValue::Int(i) = call(interp, ud_object, vec![SwiftValue::Str("n".into())])
            else {
                panic!("expected Int");
            };
            assert_eq!(i.raw, 42);
        });
    }

    #[test]
    fn set_dictionary_round_trips() {
        with_interp(true, |interp| {
            let dict = SwiftValue::Dict(Rc::new(vec![(
                SwiftValue::Str("enabled".into()),
                SwiftValue::Bool(true),
            )]));
            call(interp, ud_set, vec![dict, SwiftValue::Str("k".into())]);
            let SwiftValue::Dict(values) =
                call(interp, ud_dictionary, vec![SwiftValue::Str("k".into())])
            else {
                panic!("expected dictionary");
            };
            assert_eq!(values.len(), 1);
        });
    }

    #[test]
    fn set_int_overflowing_i64_reports_overflow_error() {
        with_interp(true, |interp| {
            // i128 but out of i64 range: i64::MAX as i128 + 1.
            let huge = SwiftValue::int(i64::MAX as i128 + 1);
            let result = ud_set(interp, standard(), vec![huge, SwiftValue::Str("k".into())]);
            let message = format!("{:?}", result.unwrap_err());
            assert!(message.contains("overflow"), "{message}");
        });
    }

    #[test]
    fn capability_gated_diagnostic_when_defaults_unavailable() {
        with_interp(false, |interp| {
            let err = ud_set(
                interp,
                standard(),
                vec![SwiftValue::Str("x".into()), SwiftValue::Str("k".into())],
            )
            .unwrap_err();
            let message = format!("{err:?}");
            assert!(message.contains("UserDefaults"), "{message}");
            assert!(
                message.contains("unavailable on this platform"),
                "{message}"
            );
        });
    }

    #[test]
    fn install_registers_host_fns_when_available() {
        with_interp(true, |_interp| {
            // `UserDefaults.standard` resolves as a static value.
            // (No direct accessor exercised here beyond install not panicking;
            // end-to-end dispatch is covered by the CLI golden fixture.)
        });
    }

    #[test]
    fn install_is_safe_when_unavailable() {
        with_interp(false, |_interp| {});
    }

    #[test]
    fn standard_singleton_identity_is_stable_across_accesses() {
        with_interp(true, |interp| {
            let a = ud_standard_static(interp, Vec::new()).unwrap();
            let b = ud_standard_static(interp, Vec::new()).unwrap();
            let (SwiftValue::Object(a), SwiftValue::Object(b)) = (a, b) else {
                panic!("expected Object");
            };
            assert!(
                Rc::ptr_eq(&a, &b),
                "UserDefaults.standard should be `===` stable across accesses"
            );
        });
    }
}
