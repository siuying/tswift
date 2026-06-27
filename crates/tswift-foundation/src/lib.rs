//! tswift-foundation — native Foundation value builtins.
//!
//! The crate mirrors the `tswift-std` registry seam: install once into an
//! interpreter, expose live `registered_keys()` for coverage tooling, and keep
//! behaviour slices small enough to validate with CLI golden fixtures.

use std::{collections::BTreeSet, rc::Rc};

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

    interp.register_free_fn("IndexPath", index_path_init);
    interp.register_property(BuiltinReceiver::IndexPath, "count", index_path_count);
    interp.register_property(BuiltinReceiver::IndexPath, "isEmpty", index_path_is_empty);
    interp.register_intrinsic(
        BuiltinReceiver::IndexPath,
        "append",
        MethodEntry {
            mutating: true,
            func: index_path_append,
        },
    );
    interp.register_intrinsic(
        BuiltinReceiver::IndexPath,
        "appending",
        MethodEntry {
            mutating: false,
            func: index_path_appending,
        },
    );

    interp.register_free_fn("IndexSet", index_set_init);
    interp.register_property(BuiltinReceiver::IndexSet, "count", index_set_count);
    interp.register_property(BuiltinReceiver::IndexSet, "isEmpty", index_set_is_empty);
    interp.register_property(BuiltinReceiver::IndexSet, "first", index_set_first);
    interp.register_property(BuiltinReceiver::IndexSet, "last", index_set_last);
    interp.register_intrinsic(
        BuiltinReceiver::IndexSet,
        "contains",
        MethodEntry {
            mutating: false,
            func: index_set_contains,
        },
    );
    interp.register_intrinsic(
        BuiltinReceiver::IndexSet,
        "insert",
        MethodEntry {
            mutating: true,
            func: index_set_insert,
        },
    );
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
            "IndexPath" => Some("IndexPath.init".to_string()),
            "IndexSet" => Some("IndexSet.init".to_string()),
            other
                if other.starts_with("Data.")
                    || other.starts_with("UUID.")
                    || other.starts_with("IndexPath.")
                    || other.starts_with("IndexSet.") =>
            {
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

fn int_arg(value: &SwiftValue, context: &str) -> Result<i128, StdError> {
    match value {
        SwiftValue::Int(i) => Ok(i.raw),
        other => Err(type_error(format!(
            "{context} expects Int, got {}",
            other.type_name()
        ))),
    }
}

fn int_array_arg(value: &SwiftValue, context: &str) -> Result<Vec<i128>, StdError> {
    match value {
        SwiftValue::Array(items) => items
            .iter()
            .map(|item| int_arg(item, context))
            .collect::<Result<Vec<_>, _>>(),
        other => Err(type_error(format!(
            "{context} expects [Int], got {}",
            other.type_name()
        ))),
    }
}

fn index_path_value(indexes: Vec<i128>) -> SwiftValue {
    let items = indexes.into_iter().map(SwiftValue::int).collect();
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "IndexPath".into(),
        fields: vec![("_indexes".into(), SwiftValue::Array(Rc::new(items)))],
    }))
}

fn index_path_indexes(value: &SwiftValue) -> Result<Vec<i128>, StdError> {
    let SwiftValue::Struct(obj) = value else {
        return Err(type_error(format!(
            "expected IndexPath, got {}",
            value.type_name()
        )));
    };
    if obj.type_name != "IndexPath" {
        return Err(type_error(format!(
            "expected IndexPath, got {}",
            obj.type_name
        )));
    }
    let Some(indexes) = obj.get("_indexes") else {
        return Err(type_error("malformed IndexPath value"));
    };
    int_array_arg(indexes, "IndexPath")
}

fn index_path_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if args.is_empty() {
        return Ok(index_path_value(Vec::new()));
    }
    if args.len() != 1 {
        return Err(type_error("IndexPath expects zero or one argument"));
    }
    match args[0].label.as_deref() {
        Some("indexes") => Ok(index_path_value(int_array_arg(
            &args[0].value,
            "IndexPath(indexes:) ",
        )?)),
        Some("index") => Ok(index_path_value(vec![int_arg(
            &args[0].value,
            "IndexPath(index:) ",
        )?])),
        Some(label) => Err(type_error(format!(
            "unsupported IndexPath argument {label}:"
        ))),
        None => Err(type_error("IndexPath argument needs a label")),
    }
}

fn index_path_count(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(index_path_indexes(&recv)?.len() as i128))
}

fn index_path_is_empty(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(index_path_indexes(&recv)?.is_empty()))
}

fn index_path_append(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if args.len() != 1 {
        return Err(type_error("IndexPath.append expects one argument"));
    }
    let mut indexes = index_path_indexes(&recv)?;
    for arg in args {
        match arg {
            SwiftValue::Struct(obj) if obj.type_name == "IndexPath" => {
                indexes.extend(index_path_indexes(&SwiftValue::Struct(obj))?);
            }
            SwiftValue::Array(_) => indexes.extend(int_array_arg(&arg, "IndexPath.append")?),
            _ => indexes.push(int_arg(&arg, "IndexPath.append")?),
        }
    }
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: index_path_value(indexes),
    })
}

fn index_path_appending(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if args.len() != 1 {
        return Err(type_error("IndexPath.appending expects one argument"));
    }
    let mut indexes = index_path_indexes(&recv)?;
    for arg in args {
        match arg {
            SwiftValue::Struct(obj) if obj.type_name == "IndexPath" => {
                indexes.extend(index_path_indexes(&SwiftValue::Struct(obj))?);
            }
            SwiftValue::Array(_) => indexes.extend(int_array_arg(&arg, "IndexPath.appending")?),
            _ => indexes.push(int_arg(&arg, "IndexPath.appending")?),
        }
    }
    Ok(Outcome {
        result: index_path_value(indexes),
        receiver: recv,
    })
}

fn index_set_value(values: BTreeSet<i128>) -> SwiftValue {
    let items = values.into_iter().map(SwiftValue::int).collect();
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "IndexSet".into(),
        fields: vec![("_values".into(), SwiftValue::Array(Rc::new(items)))],
    }))
}

fn index_set_values(value: &SwiftValue) -> Result<BTreeSet<i128>, StdError> {
    let SwiftValue::Struct(obj) = value else {
        return Err(type_error(format!(
            "expected IndexSet, got {}",
            value.type_name()
        )));
    };
    if obj.type_name != "IndexSet" {
        return Err(type_error(format!(
            "expected IndexSet, got {}",
            obj.type_name
        )));
    }
    let Some(values) = obj.get("_values") else {
        return Err(type_error("malformed IndexSet value"));
    };
    Ok(int_array_arg(values, "IndexSet")?.into_iter().collect())
}

fn ints_in_range(value: &SwiftValue, context: &str) -> Result<Vec<i128>, StdError> {
    match value {
        SwiftValue::Range { lo, hi, inclusive } => {
            let end = if *inclusive {
                hi.saturating_add(1)
            } else {
                *hi
            };
            Ok((*lo..end).collect())
        }
        other => Err(type_error(format!(
            "{context} expects Range<Int>, got {}",
            other.type_name()
        ))),
    }
}

fn index_set_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if args.is_empty() {
        return Ok(index_set_value(BTreeSet::new()));
    }
    if args.len() != 1 {
        return Err(type_error("IndexSet expects zero or one argument"));
    }
    let values = match args[0].label.as_deref() {
        Some("integer") => [int_arg(&args[0].value, "IndexSet(integer:) ")?]
            .into_iter()
            .collect(),
        Some("integersIn") => ints_in_range(&args[0].value, "IndexSet(integersIn:) ")?
            .into_iter()
            .collect(),
        Some(label) => {
            return Err(type_error(format!(
                "unsupported IndexSet argument {label}:"
            )))
        }
        None => return Err(type_error("IndexSet argument needs a label")),
    };
    Ok(index_set_value(values))
}

fn index_set_count(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::int(index_set_values(&recv)?.len() as i128))
}

fn index_set_is_empty(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(index_set_values(&recv)?.is_empty()))
}

fn index_set_first(recv: SwiftValue) -> StdResult {
    Ok(index_set_values(&recv)?
        .first()
        .copied()
        .map(SwiftValue::int)
        .unwrap_or(SwiftValue::Nil))
}

fn index_set_last(recv: SwiftValue) -> StdResult {
    Ok(index_set_values(&recv)?
        .last()
        .copied()
        .map(SwiftValue::int)
        .unwrap_or(SwiftValue::Nil))
}

fn index_set_contains(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if args.len() != 1 {
        return Err(type_error("IndexSet.contains expects one Int"));
    }
    let value = int_arg(&args[0], "IndexSet.contains")?;
    Ok(Outcome {
        result: SwiftValue::Bool(index_set_values(&recv)?.contains(&value)),
        receiver: recv,
    })
}

fn index_set_insert(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if args.len() != 1 {
        return Err(type_error("IndexSet.insert expects one Int"));
    }
    let value = int_arg(&args[0], "IndexSet.insert")?;
    let mut values = index_set_values(&recv)?;
    let inserted = values.insert(value);
    Ok(Outcome {
        result: SwiftValue::tuple_labeled(
            vec![SwiftValue::Bool(inserted), SwiftValue::int(value)],
            vec![Some("inserted".into()), Some("memberAfterInsert".into())],
        ),
        receiver: index_set_value(values),
    })
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
