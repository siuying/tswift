//! tswift-foundation — native Foundation value builtins.
//!
//! The crate mirrors the `tswift-std` registry seam: install once into an
//! interpreter, expose live `registered_keys()` for coverage tooling, and keep
//! behaviour slices small enough to validate with CLI golden fixtures.

use std::rc::Rc;

use tswift_core::{
    Arg, BuiltinReceiver, EvalError, Interpreter, MethodEntry, Outcome, StdContext, StdError,
    StdResult, StructObj, SwiftValue,
};

/// Register every currently-supported Foundation builtin into `interp`.
pub fn install(interp: &mut Interpreter<'_>) {
    interp.register_free_fn("Data", data_init);
    interp.register_property(BuiltinReceiver::Data, "count", data_count);
    interp.register_property(BuiltinReceiver::Data, "isEmpty", data_is_empty);
    interp.register_intrinsic(
        BuiltinReceiver::Data,
        "append",
        MethodEntry {
            mutating: true,
            func: data_append,
        },
    );

    interp.register_free_fn("UUID", uuid_init);
    interp.register_property(BuiltinReceiver::UUID, "uuidString", uuid_string);
}

/// Every Foundation entry registered by [`install`], as coverage keys.
pub fn registered_keys() -> Vec<String> {
    let mut sink = std::io::sink();
    let mut interp = Interpreter::new(&mut sink);
    install(&mut interp);
    let mut keys: Vec<String> = interp
        .registered_keys()
        .into_iter()
        .filter_map(|key| match key.as_str() {
            "Data" => Some("Data.init".to_string()),
            "UUID" => Some("UUID.init".to_string()),
            other if other.starts_with("Data.") || other.starts_with("UUID.") => {
                Some(other.to_string())
            }
            _ => None,
        })
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

fn type_error(message: impl Into<String>) -> StdError {
    StdError::Error(EvalError::Type(message.into()))
}

fn data_value(bytes: Vec<u8>) -> SwiftValue {
    let elements = bytes
        .into_iter()
        .map(|b| SwiftValue::int(i128::from(b)))
        .collect();
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "Data".into(),
        fields: vec![("_bytes".into(), SwiftValue::Array(Rc::new(elements)))],
    }))
}

fn data_bytes(value: &SwiftValue) -> Result<Vec<u8>, StdError> {
    let SwiftValue::Struct(obj) = value else {
        return Err(type_error(format!(
            "expected Data, got {}",
            value.type_name()
        )));
    };
    if obj.type_name != "Data" {
        return Err(type_error(format!("expected Data, got {}", obj.type_name)));
    }
    let Some(SwiftValue::Array(items)) = obj.get("_bytes") else {
        return Err(type_error("malformed Data value"));
    };
    items
        .iter()
        .map(byte_from_value)
        .collect::<Result<Vec<_>, _>>()
}

fn byte_from_value(value: &SwiftValue) -> Result<u8, StdError> {
    match value {
        SwiftValue::Int(i) if (0..=255).contains(&i.raw) => Ok(i.raw as u8),
        SwiftValue::Int(i) => Err(type_error(format!("byte value {} out of range", i.raw))),
        other => Err(type_error(format!(
            "expected UInt8 byte, got {}",
            other.type_name()
        ))),
    }
}

fn data_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if args.is_empty() {
        return Ok(data_value(Vec::new()));
    }
    if args.len() != 1 {
        return Err(type_error(
            "Data expects zero arguments or one byte sequence",
        ));
    }
    match &args[0].value {
        SwiftValue::Array(items) => {
            let bytes = items
                .iter()
                .map(byte_from_value)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(data_value(bytes))
        }
        SwiftValue::Struct(obj) if obj.type_name == "Data" => Ok(args[0].value.clone()),
        other => Err(type_error(format!(
            "Data expects [UInt8] or Data, got {}",
            other.type_name()
        ))),
    }
}

fn data_count(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(data_bytes(&recv)?.len() as i128))
}

fn data_is_empty(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(data_bytes(&recv)?.is_empty()))
}

fn data_append(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let mut bytes = data_bytes(&recv)?;
    for arg in args {
        match &arg {
            SwiftValue::Struct(obj) if obj.type_name == "Data" => bytes.extend(data_bytes(&arg)?),
            _ => bytes.push(byte_from_value(&arg)?),
        }
    }
    let receiver = data_value(bytes);
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver,
    })
}

fn uuid_value(uuid: String) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "UUID".into(),
        fields: vec![("uuidString".into(), SwiftValue::Str(uuid))],
    }))
}

fn uuid_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if args.is_empty() {
        return Ok(uuid_value(random_uuid(ctx)));
    }
    if args.len() == 1 && args[0].label.as_deref() == Some("uuidString") {
        let SwiftValue::Str(raw) = &args[0].value else {
            return Err(type_error("UUID(uuidString:) expects String"));
        };
        return Ok(match normalize_uuid(raw) {
            Some(uuid) => uuid_value(uuid),
            None => SwiftValue::Nil,
        });
    }
    Err(type_error("UUID expects no arguments or uuidString:"))
}

fn uuid_string(recv: SwiftValue) -> StdResult {
    let SwiftValue::Struct(obj) = recv else {
        return Err(type_error("uuidString expects UUID"));
    };
    if obj.type_name != "UUID" {
        return Err(type_error("uuidString expects UUID"));
    }
    match obj.get("uuidString") {
        Some(SwiftValue::Str(s)) => Ok(SwiftValue::Str(s.clone())),
        _ => Err(type_error("malformed UUID value")),
    }
}

fn normalize_uuid(raw: &str) -> Option<String> {
    let upper = raw.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    let groups = [8, 13, 18, 23];
    if bytes.len() != 36 || groups.iter().any(|&i| bytes[i] != b'-') {
        return None;
    }
    if bytes
        .iter()
        .enumerate()
        .any(|(i, b)| !groups.contains(&i) && !b.is_ascii_hexdigit())
    {
        return None;
    }
    Some(upper)
}

fn random_uuid(ctx: &mut dyn StdContext) -> String {
    let mut bytes = [0u8; 16];
    for chunk in bytes.chunks_mut(8) {
        let rand = ctx.random_u64().to_be_bytes();
        chunk.copy_from_slice(&rand[..chunk.len()]);
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

#[cfg(test)]
mod coverage_dump {
    #[test]
    fn dump_registered_keys() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = root.join("frameworks/foundation/registered_keys.txt");
        let body = super::registered_keys().join("\n") + "\n";
        std::fs::write(&path, body).expect("write registered_keys.txt");
    }
}
