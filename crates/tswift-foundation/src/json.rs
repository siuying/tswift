//! JSON-coding support for Foundation: `String.Encoding` statics,
//! `String(data:encoding:)` initializer, and strategy enums for
//! `JSONEncoder` / `JSONDecoder`.
//!
//! ## Strategy resolution (leading-dot only)
//!
//! In real Swift all strategy enums are nested types whose cases are accessed
//! via leading-dot shorthand (e.g. `enc.dateEncodingStrategy = .iso8601`).
//! The two-level form `JSONEncoder.iso8601` does NOT exist in Foundation.
//!
//! This module registers the strategy enums as **builtin enums** so that
//! `.iso8601`, `.secondsSince1970`, etc. resolve through the interpreter's
//! implicit-member mechanism.  The strategy readers in `coding.rs` accept
//! either a `SwiftValue::Enum` (from leading-dot) or a legacy `SwiftValue::Int`
//! (for backward compatibility with older test programs).
//!
//! Registered builtin enum types:
//! * `JSONEncoder.DateEncodingStrategy` — deferredToDate, secondsSince1970,
//!   millisecondsSince1970, iso8601
//! * `JSONDecoder.DateDecodingStrategy` — same cases
//! * `JSONEncoder.DataEncodingStrategy` — base64, deferredToData
//! * `JSONDecoder.DataDecodingStrategy` — same cases
//! * `JSONEncoder.KeyEncodingStrategy` — useDefaultKeys, convertToSnakeCase
//! * `JSONDecoder.KeyDecodingStrategy` — useDefaultKeys, convertFromSnakeCase
//! * `JSONEncoder.OutputFormatting` — prettyPrinted, sortedKeys
//!
//! `.iso8601` is shared with `Date.FormatStyle`; the resolver always picks
//! `Date.FormatStyle` (alphabetically first) which is fine — the strategy
//! readers dispatch on the case name, not the type name.

use tswift_core::{Arg, BuiltinParam, Interpreter, StdContext, StdError, StdResult, SwiftValue};

use crate::type_error;

/// Register JSON-coding helpers into `interp`.
pub fn install(interp: &mut Interpreter<'_>) {
    // Only `utf8` is registered: the interpreter only models UTF-8 decoding
    // and advertising `ascii`/`utf16` without implementing them would be
    // misleading (callers would silently get wrong behaviour).  The raw value 4
    // matches NS(UTF8StringEncoding); its actual integer is never inspected by
    // the runtime — only the leading-dot resolution matters.
    interp.register_static_value("String.Encoding", "utf8", SwiftValue::int(4));

    // `String(data:encoding:)` — convert a `Data` value to a `String` using
    // UTF-8.  The `encoding:` parameter carries a type hint so the interpreter
    // pushes `String.Encoding` as the contextual type while evaluating the
    // second argument, letting `.utf8` resolve as a leading-dot member.
    interp.register_free_fn_typed(
        "String",
        string_from_data_encoding,
        vec![
            BuiltinParam::labeled("data", "Data"),
            BuiltinParam::labeled("encoding", "String.Encoding"),
        ],
    );

    // ---- Strategy enums (builtin enums for leading-dot resolution) ----
    //
    // These replace the old `register_static_value("JSONEncoder", "iso8601", …)`
    // pattern. `JSONEncoder.iso8601` does not exist in real Foundation;
    // only the leading-dot `.iso8601` form is valid Swift.  The strategy
    // readers in coding.rs accept `SwiftValue::Enum { case }` produced by
    // these registrations.

    // Date strategies.
    const DATE_STRATEGY_CASES: &[&str] = &[
        "deferredToDate",
        "secondsSince1970",
        "millisecondsSince1970",
        "iso8601",
    ];
    interp.register_builtin_enum("JSONEncoder.DateEncodingStrategy", DATE_STRATEGY_CASES);
    interp.register_builtin_enum("JSONDecoder.DateDecodingStrategy", DATE_STRATEGY_CASES);

    // Data strategies.
    const DATA_STRATEGY_CASES: &[&str] = &["base64", "deferredToData"];
    interp.register_builtin_enum("JSONEncoder.DataEncodingStrategy", DATA_STRATEGY_CASES);
    interp.register_builtin_enum("JSONDecoder.DataDecodingStrategy", DATA_STRATEGY_CASES);

    // Key strategies.
    interp.register_builtin_enum(
        "JSONEncoder.KeyEncodingStrategy",
        &["useDefaultKeys", "convertToSnakeCase"],
    );
    interp.register_builtin_enum(
        "JSONDecoder.KeyDecodingStrategy",
        &["useDefaultKeys", "convertFromSnakeCase"],
    );

    // OutputFormatting (OptionSet) — treated as enum for leading-dot resolution.
    interp.register_builtin_enum(
        "JSONEncoder.OutputFormatting",
        &["prettyPrinted", "sortedKeys"],
    );
}

/// Returns the member keys exposed by this module (for coverage tracking).
pub fn registered_keys() -> Vec<String> {
    vec![
        "JSONDecoder.decode".to_string(),
        "JSONDecoder.dataDecodingStrategy".to_string(),
        "JSONDecoder.dateDecodingStrategy".to_string(),
        "JSONDecoder.keyDecodingStrategy".to_string(),
        "JSONDecoder.init".to_string(),
        "JSONEncoder.dataEncodingStrategy".to_string(),
        "JSONEncoder.dateEncodingStrategy".to_string(),
        "JSONEncoder.encode".to_string(),
        "JSONEncoder.init".to_string(),
        "JSONEncoder.keyEncodingStrategy".to_string(),
        "JSONEncoder.outputFormatting".to_string(),
        // Measurement.encode is handled via the JSON encoder's special-case path.
        "Measurement.encode".to_string(),
    ]
}

/// `String(data: Data, encoding: String.Encoding)` — failable: `nil` on invalid
/// UTF-8 or unsupported encoding (we only model UTF-8 in this runtime).
fn string_from_data_encoding(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if args.len() == 2
        && args[0].label.as_deref() == Some("data")
        && args[1].label.as_deref() == Some("encoding")
    {
        let bytes = data_bytes(&args[0].value)?;
        return Ok(match String::from_utf8(bytes) {
            Ok(s) => SwiftValue::Str(s),
            Err(_) => SwiftValue::Nil, // failable — nil on invalid UTF-8
        });
    }
    // Fall through: not the data:encoding: form; let the caller handle it.
    Err(type_error(
        "String: unsupported multi-argument initializer (only data:encoding: is implemented)",
    ))
}

/// Extract bytes from a Foundation `Data` struct value.
fn data_bytes(value: &SwiftValue) -> Result<Vec<u8>, StdError> {
    let SwiftValue::Struct(obj) = value else {
        return Err(type_error(format!(
            "String(data:encoding:) expects Data, got {}",
            value.type_name()
        )));
    };
    if obj.type_name != "Data" {
        return Err(type_error(format!(
            "String(data:encoding:) expects Data, got {}",
            obj.type_name
        )));
    }
    let Some(SwiftValue::Array(items)) = obj.get("_bytes") else {
        return Err(type_error("String(data:encoding:): malformed Data value"));
    };
    items
        .iter()
        .map(|v| match v {
            SwiftValue::Int(i) if (0..=255).contains(&i.raw) => Ok(i.raw as u8),
            SwiftValue::Int(i) => Err(type_error(format!("byte {} out of range", i.raw))),
            other => Err(type_error(format!(
                "expected UInt8 byte, got {}",
                other.type_name()
            ))),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tswift_core::Interpreter;

    #[test]
    fn string_from_utf8_data() {
        // Smoke-test: install does not panic and registered_keys is non-empty.
        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        install(&mut interp);
        let keys = registered_keys();
        assert!(keys.contains(&"JSONEncoder.init".to_string()));
        assert!(keys.contains(&"JSONEncoder.encode".to_string()));
        assert!(keys.contains(&"JSONEncoder.dateEncodingStrategy".to_string()));
        assert!(keys.contains(&"JSONDecoder.init".to_string()));
        assert!(keys.contains(&"JSONDecoder.decode".to_string()));
        assert!(keys.contains(&"JSONDecoder.dateDecodingStrategy".to_string()));
    }
}
