//! `FileManager` — Foundation's filesystem-access type, backed by the
//! `tswift.fs` host service ([`tswift_core::host_services`]).
//!
//! Also home to the file-URL-loading helpers layered on the same wire:
//! `Data(contentsOf:)`, `Data.write(to:)`/`write(toFile:)`,
//! `String(contentsOfFile:)`/`String(contentsOf:)`, and
//! `String.write(to:)`/`write(toFile:)`. `String`'s variants are dispatched
//! from `json.rs`'s single `"String"` free-fn registration (core's free-fn
//! table holds one entry per name — see that module's dispatcher) rather than
//! registered here directly.
//!
//! ## Host wire schema
//!
//! Nine host functions, all declared with stage-1 types (see
//! `tswift_core::host_bridge`). Binary content crosses the wire as a base64
//! `String` (this runtime's stage-1 vocabulary has no `Data` type):
//!
//! - `tswift.fs.exists(path: String) -> Bool`
//! - `tswift.fs.isDirectory(path: String) -> Bool` — `false` for a missing
//!   path (matches `fileExists(atPath:)` returning `false` for a missing
//!   path, extended the same way for the directory question).
//! - `tswift.fs.read(path: String) -> String?` — base64 file content, `nil`
//!   if the path does not name a readable regular file.
//! - `tswift.fs.list(path: String) -> [String]` — entry names (not full
//!   paths), throws on a missing/unreadable directory.
//! - `tswift.fs.mkdir(path: String, withIntermediateDirectories: Bool) ->
//!   Void` — throws.
//! - `tswift.fs.remove(path: String) -> Void` — throws.
//! - `tswift.fs.write(path: String, content: String, atomically: Bool) ->
//!   Bool` — `content` is base64; returns `false` on failure rather than
//!   throwing, matching `createFile(atPath:contents:)`'s `Bool` return.
//!   `atomically: true` asks the host to write via a temp-file-then-rename
//!   (or equivalent) so a concurrent reader never observes a partial write —
//!   see `String.write(to:atomically:encoding:)` below.
//! - `tswift.fs.copy(from: String, to: String) -> Void` — throws.
//! - `tswift.fs.move(from: String, to: String) -> Void` — throws.
//! - `tswift.fs.directory(kind: String) -> String` — the portable virtual
//!   root (`default`, `documents`, `caches`, or `temporary`).
//! - `tswift.fs.attributes(path: String) -> String` — JSON text for the
//!   portable basics: `size` and `isDirectory`; throws.
//!
//! Foundation only *declares* these signatures (via
//! [`tswift_core::Interpreter::register_host_fn`]) when the platform's
//! [`Capabilities`] backs [`HostService::FileSystem`]; the platform embedding
//! supplies the handler via `Interpreter::set_host_call_handler` (or a
//! per-function handler). What backs the store — the real filesystem, an
//! in-memory tree, a sandboxed root, … — is entirely the host's business.
//!
//! ## Errors
//!
//! A throwing operation that fails host-side raises a catchable Swift error
//! by returning `{"$thrown": "<message>"}` from the host handler — the same
//! mechanism every host function uses (see `tswift_core::host_bridge`'s
//! `$thrown` payload and `Interpreter::call_host_fn`'s `HostError` wrapper).
//! Filesystem failures surface as a portable `CocoaError` struct with
//! `code: Int` and `message: String`; callers can catch a matching
//! `struct CocoaError: Error { let code: Int; let message: String }`. The
//! codes cover the portable cases (`4` missing item, `512` write/general
//! failure); platform `NSError` domains, `userInfo`, permissions metadata,
//! and Darwin-only codes remain intentionally unsupported.
//!
//! ## Deviations from real Foundation
//!
//! - `fileExists(atPath:isDirectory:)` (the `inout Bool` overload) is not
//!   implemented — this runtime's intrinsic dispatch does not thread `inout`
//!   parameters through method calls. Use `fileExists(atPath:)` plus a
//!   directory check via `contentsOfDirectory(atPath:)` if both are needed.
//! - `createDirectory`/`createFile` accept (and ignore) a trailing
//!   `attributes:` argument for source compatibility; no file attributes are
//!   modelled or applied.
//! - `contentsOfDirectory(atPath:)` returns entry names in host-determined
//!   order (real Foundation's order is also unspecified, but the native CLI
//!   handler currently sorts lexically for deterministic golden fixtures —
//!   see `tswift-cli/src/fs.rs`).
//! - `String(contentsOf:)`/`Data(contentsOf:)` only accept `file:` URLs — a
//!   non-file URL throws rather than performing a network fetch (Foundation's
//!   synchronous `contentsOf:` *can* fetch a remote URL; this runtime's
//!   networking is always asynchronous, so that path is not modelled).
//! - `String(contentsOfFile:encoding:)`/`String(contentsOf:encoding:)` are
//!   not implemented — only the UTF-8-assuming, no-`encoding:` forms are
//!   (matching how `String(data:encoding:)` only models UTF-8 elsewhere in
//!   this crate).

use std::rc::Rc;

use tswift_core::json::Json;
use tswift_core::{
    Arg, BuiltinReceiver, ClassObj, HostService, Interpreter, MethodEntry, Outcome, StdContext,
    StdError, StdResult, SwiftValue,
};

use crate::url::{url_is_file_flag, url_path_string};
use crate::{data_bytes, data_value, type_error};

/// The stable [`BuiltinReceiver`] key for `FileManager`, minted once per
/// process via [`BuiltinReceiver::register_extension`] — see
/// `user_defaults.rs` for why this seam exists instead of a hardcoded
/// `BuiltinReceiver` variant.
fn receiver() -> BuiltinReceiver {
    BuiltinReceiver::register_extension("FileManager")
}

/// The [`StdContext::singleton`] cache key for `FileManager.default`.
const STANDARD_KEY: &str = "FileManager.default";

/// Register `FileManager` into `interp`. When `available` is `false` (the
/// `tswift.fs` host service is not backed by the current platform),
/// `FileManager.default` and its methods are still registered, but every
/// method body raises the capability diagnostic instead of touching the host.
pub(crate) fn install(interp: &mut Interpreter<'_>, available: bool) {
    if available {
        let _ = interp.register_host_fn(
            r#"{"name":"tswift.fs.exists","params":[{"label":"path","type":"String"}],"returns":"Bool"}"#,
            None,
        );
        let _ = interp.register_host_fn(
            r#"{"name":"tswift.fs.isDirectory","params":[{"label":"path","type":"String"}],"returns":"Bool"}"#,
            None,
        );
        let _ = interp.register_host_fn(
            r#"{"name":"tswift.fs.read","params":[{"label":"path","type":"String"}],"returns":{"optional":"String"}}"#,
            None,
        );
        let _ = interp.register_host_fn(
            r#"{"name":"tswift.fs.list","params":[{"label":"path","type":"String"}],"returns":{"array":"String"},"throws":true}"#,
            None,
        );
        let _ = interp.register_host_fn(
            r#"{"name":"tswift.fs.mkdir","params":[{"label":"path","type":"String"},{"label":"withIntermediateDirectories","type":"Bool"}],"returns":"Void","throws":true}"#,
            None,
        );
        let _ = interp.register_host_fn(
            r#"{"name":"tswift.fs.remove","params":[{"label":"path","type":"String"}],"returns":"Void","throws":true}"#,
            None,
        );
        let _ = interp.register_host_fn(
            r#"{"name":"tswift.fs.write","params":[{"label":"path","type":"String"},{"label":"content","type":"String"},{"label":"atomically","type":"Bool"}],"returns":"Bool"}"#,
            None,
        );
        let _ = interp.register_host_fn(
            r#"{"name":"tswift.fs.copy","params":[{"label":"from","type":"String"},{"label":"to","type":"String"}],"returns":"Void","throws":true}"#,
            None,
        );
        let _ = interp.register_host_fn(
            r#"{"name":"tswift.fs.move","params":[{"label":"from","type":"String"},{"label":"to","type":"String"}],"returns":"Void","throws":true}"#,
            None,
        );
        let _ = interp.register_host_fn(
            r#"{"name":"tswift.fs.directory","params":[{"label":"kind","type":"String"}],"returns":"String","throws":true}"#,
            None,
        );
        let _ = interp.register_host_fn(
            r#"{"name":"tswift.fs.attributes","params":[{"label":"path","type":"String"}],"returns":"String","throws":true}"#,
            None,
        );
    }

    // `register_static`, not `register_static_value` — same
    // bare-`.default`-shorthand collision risk documented in
    // `user_defaults.rs` for `.standard` (`URLSession.shared` and other
    // builtins also own a bare `.default`/`.shared` shorthand).
    interp.register_static(receiver(), "default", fm_default_static);

    for (name, func) in [
        ("fileExists", fm_file_exists as tswift_core::IntrinsicFn),
        ("contents", fm_contents),
        ("contentsOfDirectory", fm_contents_of_directory),
        ("createDirectory", fm_create_directory),
        ("removeItem", fm_remove_item),
        ("createFile", fm_create_file),
        ("copyItem", fm_copy_item),
        ("moveItem", fm_move_item),
        ("attributesOfItem", fm_attributes_of_item),
    ] {
        interp.register_intrinsic(
            receiver(),
            name,
            MethodEntry {
                mutating: false,
                func,
            },
        );
    }

    // File-URL-loading helpers layered on the same host functions.
    interp.register_intrinsic(
        BuiltinReceiver::Data,
        "write",
        MethodEntry {
            mutating: false,
            func: data_write_to,
        },
    );
    interp.register_intrinsic(
        BuiltinReceiver::String,
        "write",
        MethodEntry {
            mutating: false,
            func: string_write_to,
        },
    );
}

fn fm_default_static(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if !args.is_empty() {
        return Err(type_error("FileManager.default expects no arguments"));
    }
    Ok(ctx.singleton(STANDARD_KEY, file_manager_value))
}

fn file_manager_value() -> SwiftValue {
    SwiftValue::Object(Rc::new(std::cell::RefCell::new(ClassObj {
        class_name: "FileManager".into(),
        fields: Vec::new(),
    })))
}

/// The capability-gated diagnostic raised by every `FileManager` method (and
/// every file-URL-loading helper) when the host does not back
/// [`HostService::FileSystem`].
fn unavailable(api: &str) -> StdError {
    tswift_core::StdError::Error(tswift_core::EvalError::Type(
        tswift_core::CapabilityError {
            service: HostService::FileSystem,
            api: api.to_string(),
        }
        .to_string(),
    ))
}

fn require_string(value: &SwiftValue, who: &str) -> Result<String, StdError> {
    match value {
        SwiftValue::Str(s) => Ok(s.clone()),
        SwiftValue::Substring { base, start, end } => Ok(base[*start..*end].to_string()),
        other => Err(type_error(format!(
            "{who} expects a String, got {}",
            other.type_name()
        ))),
    }
}

fn require_path(args: &[SwiftValue], who: &str) -> Result<String, StdError> {
    match args.first() {
        Some(v) => require_string(v, who),
        None => Err(type_error(format!("{who} expects a path argument"))),
    }
}

// ---------------------------------------------------------------------------
// FileManager methods
// ---------------------------------------------------------------------------

fn fm_file_exists(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !ctx.is_host_fn("tswift.fs.exists") {
        return Err(unavailable("FileManager"));
    }
    let path = require_path(&args, "fileExists(atPath:)")?;
    let result = ctx.call_host_fn(
        "tswift.fs.exists",
        vec![(Some("path".to_string()), SwiftValue::Str(path))],
    )?;
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

fn fm_contents(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !ctx.is_host_fn("tswift.fs.read") {
        return Err(unavailable("FileManager"));
    }
    let path = require_path(&args, "contents(atPath:)")?;
    // `FileManager.contents(atPath:)` is non-throwing: any unreadable path
    // (missing file *or* a sandbox-escape rejection the host reports as a
    // thrown permission error) yields `nil`, not a propagated error.
    let result = read_base64(ctx, path).unwrap_or(SwiftValue::Nil);
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

/// Shared by `FileManager.contents(atPath:)` and the file-URL-loading
/// initializers: fetch base64 content for `path`, decoding it to a `Data?`
/// (`nil` when the host reports no readable file).
fn read_base64(ctx: &mut dyn StdContext, path: String) -> StdResult {
    let result = ctx.call_host_fn(
        "tswift.fs.read",
        vec![(Some("path".to_string()), SwiftValue::Str(path))],
    )?;
    match result {
        SwiftValue::Nil => Ok(SwiftValue::Nil),
        SwiftValue::Str(b64) => match tswift_core::base64::decode(&b64) {
            Some(bytes) => Ok(data_value(bytes)),
            None => Err(type_error(
                "FileManager: host `tswift.fs.read` returned invalid base64",
            )),
        },
        other => Err(type_error(format!(
            "FileManager: host `tswift.fs.read` returned {}, expected String?",
            other.type_name()
        ))),
    }
}

fn fm_contents_of_directory(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !ctx.is_host_fn("tswift.fs.list") {
        return Err(unavailable("FileManager"));
    }
    let path = require_path(&args, "contentsOfDirectory(atPath:)")?;
    let result = ctx.call_host_fn(
        "tswift.fs.list",
        vec![(Some("path".to_string()), SwiftValue::Str(path))],
    )?;
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

fn fm_create_directory(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !ctx.is_host_fn("tswift.fs.mkdir") {
        return Err(unavailable("FileManager"));
    }
    if args.len() < 2 {
        return Err(type_error(
            "createDirectory(atPath:withIntermediateDirectories:) expects at least two arguments",
        ));
    }
    let path = require_string(&args[0], "createDirectory(atPath:)")?;
    let SwiftValue::Bool(intermediate) = args[1] else {
        return Err(type_error(
            "createDirectory(atPath:withIntermediateDirectories:) expects a Bool",
        ));
    };
    // A trailing `attributes:` argument (if present) is accepted and ignored
    // — see the module docs' Deviations section.
    ctx.call_host_fn(
        "tswift.fs.mkdir",
        vec![
            (Some("path".to_string()), SwiftValue::Str(path)),
            (
                Some("withIntermediateDirectories".to_string()),
                SwiftValue::Bool(intermediate),
            ),
        ],
    )?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: recv,
    })
}

fn fm_remove_item(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !ctx.is_host_fn("tswift.fs.remove") {
        return Err(unavailable("FileManager"));
    }
    let path = require_path(&args, "removeItem(atPath:)")?;
    ctx.call_host_fn(
        "tswift.fs.remove",
        vec![(Some("path".to_string()), SwiftValue::Str(path))],
    )?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: recv,
    })
}

fn fm_create_file(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !ctx.is_host_fn("tswift.fs.write") {
        return Err(unavailable("FileManager"));
    }
    if args.len() < 2 {
        return Err(type_error(
            "createFile(atPath:contents:) expects at least two arguments",
        ));
    }
    let path = require_string(&args[0], "createFile(atPath:)")?;
    // `contents` is `Data?`; `nil` creates an empty file, matching Foundation.
    let bytes = match &args[1] {
        SwiftValue::Nil => Vec::new(),
        other => data_bytes(other)?,
    };
    // `createFile(atPath:contents:)` has no `atomically:` parameter in
    // Foundation; always non-atomic here.
    let result = write_base64(ctx, path, bytes, false)?;
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

/// Shared by `FileManager.createFile(atPath:contents:)` and the
/// file-URL-loading `write(to:)`/`write(toFile:)` methods: base64-encode
/// `bytes` and send them to `tswift.fs.write`, returning its `Bool` result.
/// `atomically` is forwarded to the host as-is — see the module docs' wire
/// schema for `tswift.fs.write`.
fn write_base64(
    ctx: &mut dyn StdContext,
    path: String,
    bytes: Vec<u8>,
    atomically: bool,
) -> StdResult {
    let content = tswift_core::base64::encode(&bytes);
    ctx.call_host_fn(
        "tswift.fs.write",
        vec![
            (Some("path".to_string()), SwiftValue::Str(path)),
            (Some("content".to_string()), SwiftValue::Str(content)),
            (Some("atomically".to_string()), SwiftValue::Bool(atomically)),
        ],
    )
}

fn fm_copy_item(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !ctx.is_host_fn("tswift.fs.copy") {
        return Err(unavailable("FileManager"));
    }
    if args.len() != 2 {
        return Err(type_error("copyItem(atPath:toPath:) expects two arguments"));
    }
    let from = require_string(&args[0], "copyItem(atPath:)")?;
    let to = require_string(&args[1], "copyItem(toPath:)")?;
    ctx.call_host_fn(
        "tswift.fs.copy",
        vec![
            (Some("from".to_string()), SwiftValue::Str(from)),
            (Some("to".to_string()), SwiftValue::Str(to)),
        ],
    )?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: recv,
    })
}

fn fm_move_item(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !ctx.is_host_fn("tswift.fs.move") {
        return Err(unavailable("FileManager"));
    }
    if args.len() != 2 {
        return Err(type_error("moveItem(atPath:toPath:) expects two arguments"));
    }
    let from = require_string(&args[0], "moveItem(atPath:)")?;
    let to = require_string(&args[1], "moveItem(toPath:)")?;
    ctx.call_host_fn(
        "tswift.fs.move",
        vec![
            (Some("from".to_string()), SwiftValue::Str(from)),
            (Some("to".to_string()), SwiftValue::Str(to)),
        ],
    )?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: recv,
    })
}

fn fm_attributes_of_item(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !ctx.is_host_fn("tswift.fs.attributes") {
        return Err(unavailable("FileManager"));
    }
    let path = require_path(&args, "attributesOfItem(atPath:)")?;
    let value = ctx.call_host_fn(
        "tswift.fs.attributes",
        vec![(Some("path".to_string()), SwiftValue::Str(path))],
    )?;
    let SwiftValue::Str(document) = value else {
        return Err(type_error(
            "FileManager: host `tswift.fs.attributes` returned non-String",
        ));
    };
    let Json::Object(entries) = tswift_core::json::parse(&document)
        .map_err(|e| type_error(format!("FileManager: invalid attributes JSON: {e}")))?
    else {
        return Err(type_error("FileManager: attributes were not a dictionary"));
    };
    let values = entries
        .into_iter()
        .filter_map(|(key, value)| match value {
            Json::Int(value) => Some((SwiftValue::Str(key), SwiftValue::int(i128::from(value)))),
            Json::Bool(value) => Some((SwiftValue::Str(key), SwiftValue::Bool(value))),
            _ => None,
        })
        .collect();
    Ok(Outcome {
        result: SwiftValue::Dict(Rc::new(values)),
        receiver: recv,
    })
}

// ---------------------------------------------------------------------------
// File-URL-loading helpers
// ---------------------------------------------------------------------------

/// Resolve a `contentsOf`/`to` argument (a `URL`) to a filesystem path,
/// requiring it to be a `file:` URL — see the module docs' Deviations
/// section on why non-file URLs are rejected rather than fetched.
///
/// A wrong-*type* argument (not a `URL` at all) is a genuine programming
/// error and stays a non-catchable [`type_error`]. A well-typed `URL` that
/// simply isn't a `file:` URL, though, mirrors Foundation's throwing
/// `contentsOf:`/`write(to:)` initializers — that failure must be a
/// catchable Swift error, not a trap, so it raises the portable Cocoa error
/// shape every other host-detected failure in this module uses.
fn file_url_path(
    ctx: &mut dyn StdContext,
    value: &SwiftValue,
    who: &str,
) -> Result<String, StdError> {
    let SwiftValue::Struct(obj) = value else {
        return Err(type_error(format!(
            "{who} expects a URL, got {}",
            value.type_name()
        )));
    };
    if obj.type_name != "URL" {
        return Err(type_error(format!(
            "{who} expects a URL, got {}",
            obj.type_name
        )));
    }
    if !url_is_file_flag(value) {
        return Err(ctx.throw(cocoa_error(
            512,
            &format!("{who}: only `file:` URLs are supported by this runtime"),
        )));
    }
    url_path_string(value)
}

/// `Data(contentsOf: URL)` — called from `data_init`'s `contentsOf` branch.
pub(crate) fn data_contents_of(ctx: &mut dyn StdContext, url: &SwiftValue) -> StdResult {
    if !ctx.is_host_fn("tswift.fs.read") {
        return Err(unavailable("Data(contentsOf:)"));
    }
    let path = file_url_path(ctx, url, "Data(contentsOf:)")?;
    match read_base64(ctx, path)? {
        SwiftValue::Nil => Err(ctx.throw(cocoa_error(
            4,
            "The file couldn\u{2019}t be opened because there is no such file.",
        ))),
        data => Ok(data),
    }
}

/// `String(contentsOfFile:)` / `String(contentsOf:)` — called from
/// `json.rs`'s `"String"` free-fn dispatcher.
pub(crate) fn string_contents_of(
    ctx: &mut dyn StdContext,
    label: &str,
    value: &SwiftValue,
) -> StdResult {
    if !ctx.is_host_fn("tswift.fs.read") {
        return Err(unavailable("String(contentsOf:)"));
    }
    let path = match label {
        "contentsOfFile" => require_string(value, "String(contentsOfFile:)")?,
        "contentsOf" => file_url_path(ctx, value, "String(contentsOf:)")?,
        _ => unreachable!("string_contents_of called with unexpected label {label}"),
    };
    match read_base64(ctx, path)? {
        SwiftValue::Nil => Err(ctx.throw(cocoa_error(
            4,
            "The file couldn\u{2019}t be opened because there is no such file.",
        ))),
        data => {
            let bytes = data_bytes(&data)?;
            String::from_utf8(bytes).map(SwiftValue::Str).map_err(|_| {
                ctx.throw(cocoa_error(
                    486,
                    "The file couldn\u{2019}t be decoded as UTF-8.",
                ))
            })
        }
    }
}

/// `Data.write(to: URL)` / `write(toFile: String)`.
fn data_write_to(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !ctx.is_host_fn("tswift.fs.write") {
        return Err(unavailable("Data.write(to:)"));
    }
    let path = match args.first() {
        Some(v @ SwiftValue::Struct(obj)) if obj.type_name == "URL" => {
            file_url_path(ctx, v, "Data.write(to:)")?
        }
        Some(other) => require_string(other, "Data.write(toFile:)")?,
        None => return Err(type_error("Data.write(to:) expects a destination argument")),
    };
    let bytes = data_bytes(&recv)?;
    // `Data.write(to:options:)` doesn't take a labelled `atomically:` Bool in
    // Foundation (it takes `options: Data.WritingOptions`, not modelled here
    // — see the module docs' Deviations section); always non-atomic.
    match write_base64(ctx, path, bytes, false)? {
        SwiftValue::Bool(true) => Ok(Outcome {
            result: SwiftValue::Void,
            receiver: recv,
        }),
        _ => Err(ctx.throw(cocoa_error(512, "The file couldn\u{2019}t be written."))),
    }
}

/// The `String.Encoding` raw value this runtime models — see `json.rs`,
/// which is the sole place `String.Encoding.utf8` is registered.
const UTF8_ENCODING: i128 = 4;

/// `String.write(to: URL, atomically: Bool, encoding: String.Encoding)` /
/// `write(toFile: String, atomically: Bool, encoding: String.Encoding)`.
/// Argument *labels* aren't validated by the interpreter's plain intrinsic
/// dispatch (see `dispatch.rs`), so `atomically`/`encoding` are read
/// positionally, matching Foundation's fixed parameter order.
fn string_write_to(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !ctx.is_host_fn("tswift.fs.write") {
        return Err(unavailable("String.write(to:)"));
    }
    let path = match args.first() {
        Some(v @ SwiftValue::Struct(obj)) if obj.type_name == "URL" => {
            file_url_path(ctx, v, "String.write(to:)")?
        }
        Some(other) => require_string(other, "String.write(toFile:)")?,
        None => {
            return Err(type_error(
                "String.write(to:) expects a destination argument",
            ))
        }
    };
    let atomically = match args.get(1) {
        Some(SwiftValue::Bool(b)) => *b,
        Some(other) => {
            return Err(type_error(format!(
                "String.write(to:atomically:) expects a Bool, got {}",
                other.type_name()
            )))
        }
        None => false,
    };
    // Only `.utf8` is modelled (see `json.rs`'s `String.Encoding` — the sole
    // case it registers); any other encoding value errors rather than being
    // silently ignored.
    if let Some(encoding) = args.get(2) {
        let is_utf8 = matches!(encoding, SwiftValue::Int(i) if i.raw == UTF8_ENCODING);
        if !is_utf8 {
            return Err(type_error(
                "String.write(to:atomically:encoding:) only supports .utf8",
            ));
        }
    }
    let text = require_string(&recv, "String.write(to:)")?;
    match write_base64(ctx, path, text.into_bytes(), atomically)? {
        SwiftValue::Bool(true) => Ok(Outcome {
            result: SwiftValue::Void,
            receiver: recv,
        }),
        _ => Err(ctx.throw(cocoa_error(512, "The file couldn\u{2019}t be written."))),
    }
}

/// Build the same portable Cocoa-shaped error used by the filesystem host for
/// failures detected after a non-throwing wire reply (`nil`/`false`).
fn cocoa_error(code: i128, message: &str) -> SwiftValue {
    SwiftValue::Struct(Rc::new(tswift_core::StructObj {
        type_name: "CocoaError".into(),
        fields: vec![
            ("code".into(), SwiftValue::int(code)),
            ("message".into(), SwiftValue::Str(message.to_string())),
        ],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use tswift_core::json::{self, Json};
    use tswift_core::{HostCallHandler, Interpreter};

    /// A real [`HostCallHandler`] backing `tswift.fs.*` with an in-memory
    /// path→bytes map (paths are opaque strings here; no real filesystem
    /// semantics like directories are modelled beyond a `dirs` set) — mirrors
    /// `user_defaults.rs`'s `DefaultsHandler` test fixture, driving the full
    /// wire through `Interpreter::set_host_call_handler`.
    struct FsHandler {
        files: Mutex<HashMap<String, Vec<u8>>>,
        dirs: Mutex<std::collections::HashSet<String>>,
    }

    impl FsHandler {
        fn new() -> Self {
            Self {
                files: Mutex::new(HashMap::new()),
                dirs: Mutex::new(std::collections::HashSet::new()),
            }
        }

        fn thrown(message: &str) -> String {
            json::to_string(&Json::Object(vec![(
                "$thrown".to_string(),
                Json::Str(message.to_string()),
            )]))
        }
    }

    impl HostCallHandler for FsHandler {
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
            let bool_arg = |i: usize| -> Result<bool, String> {
                match items.get(i) {
                    Some(Json::Bool(b)) => Ok(*b),
                    _ => Err(format!("{name}: expected a Bool argument at index {i}")),
                }
            };
            let files = &self.files;
            let dirs = &self.dirs;
            match name {
                "tswift.fs.exists" => {
                    let path = str_arg(0)?;
                    let exists = files.lock().unwrap().contains_key(&path)
                        || dirs.lock().unwrap().contains(&path);
                    Ok(json::to_string(&Json::Bool(exists)))
                }
                "tswift.fs.isDirectory" => Ok(json::to_string(&Json::Bool(
                    dirs.lock().unwrap().contains(&str_arg(0)?),
                ))),
                "tswift.fs.read" => {
                    let path = str_arg(0)?;
                    Ok(match files.lock().unwrap().get(&path) {
                        Some(bytes) => {
                            json::to_string(&Json::Str(tswift_core::base64::encode(bytes)))
                        }
                        None => "null".to_string(),
                    })
                }
                "tswift.fs.list" => {
                    let path = str_arg(0)?;
                    if !dirs.lock().unwrap().contains(&path) {
                        return Ok(Self::thrown(&format!(
                            "The folder \u{201c}{path}\u{201d} doesn\u{2019}t exist."
                        )));
                    }
                    let prefix = format!("{path}/");
                    let mut names: Vec<String> = files
                        .lock()
                        .unwrap()
                        .keys()
                        .filter_map(|k| k.strip_prefix(&prefix).map(|s| s.to_string()))
                        .filter(|s| !s.contains('/'))
                        .collect();
                    names.sort();
                    Ok(json::to_string(&Json::Array(
                        names.into_iter().map(Json::Str).collect(),
                    )))
                }
                "tswift.fs.mkdir" => {
                    let path = str_arg(0)?;
                    let _intermediate = bool_arg(1)?;
                    dirs.lock().unwrap().insert(path);
                    Ok("null".to_string())
                }
                "tswift.fs.remove" => {
                    let path = str_arg(0)?;
                    let mut removed = files.lock().unwrap().remove(&path).is_some();
                    removed |= dirs.lock().unwrap().remove(&path);
                    if !removed {
                        return Ok(Self::thrown(&format!(
                            "The file \u{201c}{path}\u{201d} doesn\u{2019}t exist."
                        )));
                    }
                    Ok("null".to_string())
                }
                "tswift.fs.write" => {
                    let path = str_arg(0)?;
                    let content = str_arg(1)?;
                    match tswift_core::base64::decode(&content) {
                        Some(bytes) => {
                            files.lock().unwrap().insert(path, bytes);
                            Ok(json::to_string(&Json::Bool(true)))
                        }
                        None => Ok(json::to_string(&Json::Bool(false))),
                    }
                }
                "tswift.fs.copy" => {
                    let from = str_arg(0)?;
                    let to = str_arg(1)?;
                    let bytes = files.lock().unwrap().get(&from).cloned();
                    match bytes {
                        Some(b) => {
                            files.lock().unwrap().insert(to, b);
                            Ok("null".to_string())
                        }
                        None => Ok(Self::thrown(&format!(
                            "The file \u{201c}{from}\u{201d} doesn\u{2019}t exist."
                        ))),
                    }
                }
                "tswift.fs.move" => {
                    let from = str_arg(0)?;
                    let to = str_arg(1)?;
                    let bytes = files.lock().unwrap().remove(&from);
                    match bytes {
                        Some(b) => {
                            files.lock().unwrap().insert(to, b);
                            Ok("null".to_string())
                        }
                        None => Ok(Self::thrown(&format!(
                            "The file \u{201c}{from}\u{201d} doesn\u{2019}t exist."
                        ))),
                    }
                }
                other => Err(format!("unexpected host fn {other}")),
            }
        }
    }

    fn with_interp<R>(available: bool, f: impl FnOnce(&mut Interpreter) -> R) -> R {
        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        if available {
            interp.set_host_call_handler(Arc::new(FsHandler::new()));
        }
        install(&mut interp, available);
        f(&mut interp)
    }

    fn default_recv() -> SwiftValue {
        file_manager_value()
    }

    fn call(
        ctx: &mut dyn StdContext,
        method: tswift_core::IntrinsicFn,
        args: Vec<SwiftValue>,
    ) -> Result<SwiftValue, StdError> {
        method(ctx, default_recv(), args).map(|o| o.result)
    }

    #[test]
    fn file_exists_false_when_absent() {
        with_interp(true, |interp| {
            assert_eq!(
                call(interp, fm_file_exists, vec![SwiftValue::Str("/x".into())]).unwrap(),
                SwiftValue::Bool(false)
            );
        });
    }

    #[test]
    fn create_file_and_exists_and_contents_round_trip() {
        with_interp(true, |interp| {
            let data = data_value(b"hello".to_vec());
            let ok = call(
                interp,
                fm_create_file,
                vec![SwiftValue::Str("/a.txt".into()), data],
            )
            .unwrap();
            assert_eq!(ok, SwiftValue::Bool(true));
            assert_eq!(
                call(
                    interp,
                    fm_file_exists,
                    vec![SwiftValue::Str("/a.txt".into())]
                )
                .unwrap(),
                SwiftValue::Bool(true)
            );
            let contents =
                call(interp, fm_contents, vec![SwiftValue::Str("/a.txt".into())]).unwrap();
            assert_eq!(data_bytes(&contents).unwrap(), b"hello".to_vec());
        });
    }

    #[test]
    fn create_file_with_nil_contents_creates_empty_file() {
        with_interp(true, |interp| {
            call(
                interp,
                fm_create_file,
                vec![SwiftValue::Str("/empty.txt".into()), SwiftValue::Nil],
            )
            .unwrap();
            let contents = call(
                interp,
                fm_contents,
                vec![SwiftValue::Str("/empty.txt".into())],
            )
            .unwrap();
            assert_eq!(data_bytes(&contents).unwrap(), Vec::<u8>::new());
        });
    }

    #[test]
    fn contents_of_missing_file_is_nil() {
        with_interp(true, |interp| {
            assert_eq!(
                call(
                    interp,
                    fm_contents,
                    vec![SwiftValue::Str("/missing".into())]
                )
                .unwrap(),
                SwiftValue::Nil
            );
        });
    }

    #[test]
    fn create_directory_and_list_contents() {
        with_interp(true, |interp| {
            call(
                interp,
                fm_create_directory,
                vec![SwiftValue::Str("/dir".into()), SwiftValue::Bool(true)],
            )
            .unwrap();
            call(
                interp,
                fm_create_file,
                vec![
                    SwiftValue::Str("/dir/a.txt".into()),
                    data_value(b"a".to_vec()),
                ],
            )
            .unwrap();
            call(
                interp,
                fm_create_file,
                vec![
                    SwiftValue::Str("/dir/b.txt".into()),
                    data_value(b"b".to_vec()),
                ],
            )
            .unwrap();
            let SwiftValue::Array(names) = call(
                interp,
                fm_contents_of_directory,
                vec![SwiftValue::Str("/dir".into())],
            )
            .unwrap() else {
                panic!("expected Array");
            };
            let names: Vec<String> = names
                .iter()
                .map(|v| match v {
                    SwiftValue::Str(s) => s.clone(),
                    other => panic!("expected String, got {other:?}"),
                })
                .collect();
            assert_eq!(names, vec!["a.txt".to_string(), "b.txt".to_string()]);
        });
    }

    #[test]
    fn contents_of_directory_missing_throws() {
        with_interp(true, |interp| {
            let err = call(
                interp,
                fm_contents_of_directory,
                vec![SwiftValue::Str("/nope".into())],
            )
            .unwrap_err();
            assert!(matches!(err, StdError::Throw(_)), "{err:?}");
        });
    }

    #[test]
    fn remove_item_deletes_file() {
        with_interp(true, |interp| {
            call(
                interp,
                fm_create_file,
                vec![SwiftValue::Str("/x".into()), data_value(vec![1])],
            )
            .unwrap();
            call(interp, fm_remove_item, vec![SwiftValue::Str("/x".into())]).unwrap();
            assert_eq!(
                call(interp, fm_file_exists, vec![SwiftValue::Str("/x".into())]).unwrap(),
                SwiftValue::Bool(false)
            );
        });
    }

    #[test]
    fn remove_item_missing_throws() {
        with_interp(true, |interp| {
            let err = call(
                interp,
                fm_remove_item,
                vec![SwiftValue::Str("/nope".into())],
            )
            .unwrap_err();
            assert!(matches!(err, StdError::Throw(_)), "{err:?}");
        });
    }

    #[test]
    fn copy_item_duplicates_content() {
        with_interp(true, |interp| {
            call(
                interp,
                fm_create_file,
                vec![SwiftValue::Str("/src".into()), data_value(b"c".to_vec())],
            )
            .unwrap();
            call(
                interp,
                fm_copy_item,
                vec![
                    SwiftValue::Str("/src".into()),
                    SwiftValue::Str("/dst".into()),
                ],
            )
            .unwrap();
            let contents = call(interp, fm_contents, vec![SwiftValue::Str("/dst".into())]).unwrap();
            assert_eq!(data_bytes(&contents).unwrap(), b"c".to_vec());
            // Source is untouched by a copy.
            assert_eq!(
                call(interp, fm_file_exists, vec![SwiftValue::Str("/src".into())]).unwrap(),
                SwiftValue::Bool(true)
            );
        });
    }

    #[test]
    fn move_item_relocates_content() {
        with_interp(true, |interp| {
            call(
                interp,
                fm_create_file,
                vec![SwiftValue::Str("/src".into()), data_value(b"m".to_vec())],
            )
            .unwrap();
            call(
                interp,
                fm_move_item,
                vec![
                    SwiftValue::Str("/src".into()),
                    SwiftValue::Str("/dst2".into()),
                ],
            )
            .unwrap();
            assert_eq!(
                call(interp, fm_file_exists, vec![SwiftValue::Str("/src".into())]).unwrap(),
                SwiftValue::Bool(false)
            );
            let contents =
                call(interp, fm_contents, vec![SwiftValue::Str("/dst2".into())]).unwrap();
            assert_eq!(data_bytes(&contents).unwrap(), b"m".to_vec());
        });
    }

    #[test]
    fn capability_gated_diagnostic_when_fs_unavailable() {
        with_interp(false, |interp| {
            let err = call(interp, fm_file_exists, vec![SwiftValue::Str("/x".into())]).unwrap_err();
            let message = format!("{err:?}");
            assert!(message.contains("FileManager"), "{message}");
            assert!(
                message.contains("unavailable on this platform"),
                "{message}"
            );
        });
    }

    #[test]
    fn default_singleton_identity_is_stable_across_accesses() {
        with_interp(true, |interp| {
            let a = fm_default_static(interp, Vec::new()).unwrap();
            let b = fm_default_static(interp, Vec::new()).unwrap();
            let (SwiftValue::Object(a), SwiftValue::Object(b)) = (a, b) else {
                panic!("expected Object");
            };
            assert!(
                Rc::ptr_eq(&a, &b),
                "FileManager.default should be `===` stable across accesses"
            );
        });
    }

    #[test]
    fn data_contents_of_file_url_reads_bytes() {
        with_interp(true, |interp| {
            call(
                interp,
                fm_create_file,
                vec![
                    SwiftValue::Str("/u.txt".into()),
                    data_value(b"url".to_vec()),
                ],
            )
            .unwrap();
            let url = crate::url::url_value("file:///u.txt".to_string());
            let data = data_contents_of(interp, &url).unwrap();
            assert_eq!(data_bytes(&data).unwrap(), b"url".to_vec());
        });
    }

    #[test]
    fn data_contents_of_missing_file_throws() {
        with_interp(true, |interp| {
            let url = crate::url::url_value("file:///missing.txt".to_string());
            let err = data_contents_of(interp, &url).unwrap_err();
            assert!(matches!(err, StdError::Throw(_)), "{err:?}");
        });
    }

    /// A non-`file:` URL mirrors Foundation's throwing `contentsOf:`
    /// initializer: it's a *catchable* Swift error (`StdError::Throw`), not
    /// a `type_error` trap — Swift code can `catch let e as HostError` it,
    /// same as any other host-detected FileManager failure.
    #[test]
    fn data_contents_of_non_file_url_is_catchable() {
        with_interp(true, |interp| {
            let url = crate::url::url_value("https://example.com/x".to_string());
            let err = data_contents_of(interp, &url).unwrap_err();
            assert!(matches!(err, StdError::Throw(_)), "{err:?}");
        });
    }

    /// Passing something that isn't a `URL` at all is a genuine programming
    /// error, not a runtime file-access failure — that stays a non-catchable
    /// type error.
    #[test]
    fn data_contents_of_wrong_type_is_type_error() {
        with_interp(true, |interp| {
            let err = data_contents_of(interp, &SwiftValue::Str("not a url".into())).unwrap_err();
            assert!(!matches!(err, StdError::Throw(_)), "{err:?}");
        });
    }

    #[test]
    fn string_contents_of_file_reads_utf8() {
        with_interp(true, |interp| {
            call(
                interp,
                fm_create_file,
                vec![
                    SwiftValue::Str("/s.txt".into()),
                    data_value(b"hi there".to_vec()),
                ],
            )
            .unwrap();
            let s = string_contents_of(interp, "contentsOfFile", &SwiftValue::Str("/s.txt".into()))
                .unwrap();
            assert_eq!(s, SwiftValue::Str("hi there".into()));
        });
    }

    #[test]
    fn data_write_to_file_url_persists() {
        with_interp(true, |interp| {
            let url = crate::url::url_value("file:///w.bin".to_string());
            let outcome =
                data_write_to(interp, data_value(b"payload".to_vec()), vec![url.clone()]).unwrap();
            assert_eq!(outcome.result, SwiftValue::Void);
            let data = data_contents_of(interp, &url).unwrap();
            assert_eq!(data_bytes(&data).unwrap(), b"payload".to_vec());
        });
    }

    #[test]
    fn string_write_to_file_path_persists() {
        with_interp(true, |interp| {
            string_write_to(
                interp,
                SwiftValue::Str("written".into()),
                vec![SwiftValue::Str("/w.txt".into())],
            )
            .unwrap();
            let s = string_contents_of(interp, "contentsOfFile", &SwiftValue::Str("/w.txt".into()))
                .unwrap();
            assert_eq!(s, SwiftValue::Str("written".into()));
        });
    }

    #[test]
    fn string_write_to_honors_atomically_flag() {
        with_interp(true, |interp| {
            string_write_to(
                interp,
                SwiftValue::Str("written".into()),
                vec![
                    SwiftValue::Str("/atomic.txt".into()),
                    SwiftValue::Bool(true),
                ],
            )
            .unwrap();
            let s = string_contents_of(
                interp,
                "contentsOfFile",
                &SwiftValue::Str("/atomic.txt".into()),
            )
            .unwrap();
            assert_eq!(s, SwiftValue::Str("written".into()));
        });
    }

    #[test]
    fn string_write_to_utf8_encoding_succeeds() {
        with_interp(true, |interp| {
            let outcome = string_write_to(
                interp,
                SwiftValue::Str("written".into()),
                vec![
                    SwiftValue::Str("/enc.txt".into()),
                    SwiftValue::Bool(false),
                    SwiftValue::int(UTF8_ENCODING),
                ],
            );
            assert!(outcome.is_ok(), "{outcome:?}");
        });
    }

    #[test]
    fn string_write_to_unsupported_encoding_errors() {
        with_interp(true, |interp| {
            let err = string_write_to(
                interp,
                SwiftValue::Str("written".into()),
                vec![
                    SwiftValue::Str("/enc.txt".into()),
                    SwiftValue::Bool(false),
                    SwiftValue::int(1), // NSASCIIStringEncoding — not modelled.
                ],
            )
            .unwrap_err();
            assert!(!matches!(err, StdError::Throw(_)), "{err:?}");
        });
    }

    #[test]
    fn install_is_safe_when_unavailable() {
        with_interp(false, |_interp| {});
    }
}
