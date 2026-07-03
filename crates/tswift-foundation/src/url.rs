//! Foundation `URL`, `URLComponents`, and `URLQueryItem`.
//!
//! All three are modelled as `SwiftValue::Struct`:
//! - `URL` stores its absolute string in `_string` plus an `_isFile` flag; every
//!   accessor parses that string on demand (RFC-3986-ish).
//! - `URLComponents` stores each component as a public-named field (`scheme`,
//!   `host`, …) so reads/writes flow through the generic struct member path;
//!   `url`/`string` are computed read-only properties.
//! - `URLQueryItem` is a plain `{ name, value }` struct.

use std::rc::Rc;

use tswift_core::{
    Arg, BuiltinReceiver, LabeledMethodEntry, MethodEntry, Outcome, PropertySetterFn, StdContext,
    StdError, StdResult, StructObj, SwiftValue,
};

use crate::type_error;

/// Register the URL family on `interp`.
#[allow(clippy::too_many_lines)]
pub(crate) fn install(interp: &mut tswift_core::Interpreter<'_>) {
    // ---- URL ----
    interp.register_free_fn("URL", url_init);
    for (name, f) in [
        (
            "absoluteString",
            url_absolute_string as fn(SwiftValue) -> StdResult,
        ),
        ("scheme", url_scheme),
        ("host", url_host),
        ("port", url_port),
        ("path", url_path),
        ("query", url_query),
        ("fragment", url_fragment),
        ("user", url_user),
        ("password", url_password),
        ("lastPathComponent", url_last_path_component),
        ("pathExtension", url_path_extension),
        ("pathComponents", url_path_components),
        ("isFileURL", url_is_file),
        ("description", url_absolute_string),
        ("debugDescription", url_absolute_string),
        ("hashValue", url_hash_value),
        // No base URL is modelled, so relative views equal the absolute ones.
        ("relativeString", url_absolute_string),
        ("relativePath", url_path),
        ("baseURL", url_base_url),
        ("hasDirectoryPath", url_has_directory_path),
        ("standardized", url_standardized),
        ("absoluteURL", url_absolute_url),
    ] {
        interp.register_property(BuiltinReceiver::URL, name, f);
    }
    for (name, mutating, f) in [
        (
            "appendingPathComponent",
            false,
            url_appending_path_component as tswift_core::IntrinsicFn,
        ),
        (
            "deletingLastPathComponent",
            false,
            url_deleting_last_path_component,
        ),
        (
            "appendingPathExtension",
            false,
            url_appending_path_extension,
        ),
        ("deletingPathExtension", false, url_deleting_path_extension),
        ("appendPathComponent", true, url_append_path_component),
        (
            "deleteLastPathComponent",
            true,
            url_delete_last_path_component,
        ),
        ("appendPathExtension", true, url_append_path_extension),
        ("deletePathExtension", true, url_delete_path_extension),
        ("standardize", true, url_standardize),
        (
            "resolvingSymlinksInPath",
            false,
            url_resolving_symlinks_in_path,
        ),
        ("resolveSymlinksInPath", true, url_resolve_symlinks_in_path),
    ] {
        interp.register_intrinsic(
            BuiltinReceiver::URL,
            name,
            MethodEntry { mutating, func: f },
        );
    }

    // `appending(path:)` / `appending(component:)` — label-sensitive
    interp.register_labeled_intrinsic(
        BuiltinReceiver::URL,
        "appending",
        LabeledMethodEntry {
            mutating: false,
            func: url_appending_labeled,
        },
    );
    // `append(path:)` / `append(component:)` — mutating, label-sensitive
    interp.register_labeled_intrinsic(
        BuiltinReceiver::URL,
        "append",
        LabeledMethodEntry {
            mutating: true,
            func: url_append_labeled,
        },
    );

    // ---- URLQueryItem ----
    interp.register_free_fn("URLQueryItem", url_query_item_init);
    interp.register_property(BuiltinReceiver::URLQueryItem, "name", url_query_item_name);
    interp.register_property(
        BuiltinReceiver::URLQueryItem,
        "value",
        url_query_item_value_prop,
    );
    interp.register_property(
        BuiltinReceiver::URLQueryItem,
        "description",
        url_query_item_description,
    );
    interp.register_property(
        BuiltinReceiver::URLQueryItem,
        "hashValue",
        url_query_item_hash_value,
    );

    // ---- URLComponents ----
    interp.register_free_fn("URLComponents", url_components_init);
    interp.register_property(BuiltinReceiver::URLComponents, "url", url_components_url);
    interp.register_property(
        BuiltinReceiver::URLComponents,
        "string",
        url_components_string,
    );
    interp.register_property(
        BuiltinReceiver::URLComponents,
        "queryItems",
        url_components_query_items,
    );
    // Component getters reading the stored fields (so coverage credits them and
    // they read uniformly even where the stored field already shadows).
    for (name, getter) in URL_COMPONENT_GETTERS {
        interp.register_property(BuiltinReceiver::URLComponents, name, *getter);
    }
    interp.register_property(
        BuiltinReceiver::URLComponents,
        "description",
        url_components_description,
    );
    interp.register_property(
        BuiltinReceiver::URLComponents,
        "debugDescription",
        url_components_description,
    );
    interp.register_property(
        BuiltinReceiver::URLComponents,
        "hashValue",
        url_components_hash_value,
    );
    // Percent-encoded getters.
    for (name, getter) in URL_COMPONENTS_ENCODED_GETTERS {
        interp.register_property(BuiltinReceiver::URLComponents, name, *getter);
    }
    // Percent-encoded setters: validate encoding, then store decoded value.
    for (name, setter) in URL_COMPONENTS_ENCODED_SETTERS {
        interp.register_property_setter(BuiltinReceiver::URLComponents, name, *setter);
    }
}

fn url_hash_value(recv: SwiftValue) -> StdResult {
    // `URL` stores only its absolute string, so structural `==` reduces to that
    // string; hashing it keeps `hashValue` consistent with equality.
    Ok(SwiftValue::int(crate::fnv1a_hash(
        url_string(&recv)?.as_bytes(),
    )))
}

fn url_query_item_hash_value(recv: SwiftValue) -> StdResult {
    let SwiftValue::Struct(o) = &recv else {
        return Err(type_error("hashValue expects URLQueryItem"));
    };
    let name = match o.get("name") {
        Some(SwiftValue::Str(s)) => s.to_string(),
        _ => String::new(),
    };
    // `value` is optional; distinguish nil from empty with a marker byte.
    let mut bytes = name.into_bytes();
    bytes.push(0);
    match o.get("value") {
        Some(SwiftValue::Str(s)) => bytes.extend_from_slice(s.as_bytes()),
        _ => bytes.push(1),
    }
    Ok(SwiftValue::int(crate::fnv1a_hash(&bytes)))
}

fn url_components_hash_value(recv: SwiftValue) -> StdResult {
    // Hash the reconstructed URL string, consistent with structural equality of
    // equal components.
    let string = match url_components_string(recv)? {
        SwiftValue::Str(s) => s.to_string(),
        _ => String::new(),
    };
    Ok(SwiftValue::int(crate::fnv1a_hash(string.as_bytes())))
}

macro_rules! url_component_getters {
    ($($field:literal => $getter:ident),+ $(,)?) => {
        $(
            fn $getter(recv: SwiftValue) -> StdResult {
                Ok(comp_field(&recv, $field).cloned().unwrap_or(SwiftValue::Nil))
            }
        )+
        const URL_COMPONENT_GETTERS: &[(&str, tswift_core::PropertyFn)] = &[
            $(($field, $getter)),+
        ];
    };
}

url_component_getters! {
    "scheme" => url_components_scheme,
    "host" => url_components_host,
    "port" => url_components_port,
    "path" => url_components_path,
    "user" => url_components_user,
    "password" => url_components_password,
    "query" => url_components_query,
    "fragment" => url_components_fragment,
}

fn url_query_item_name(recv: SwiftValue) -> StdResult {
    match &recv {
        SwiftValue::Struct(o) if o.type_name == "URLQueryItem" => {
            Ok(o.get("name").cloned().unwrap_or(SwiftValue::Nil))
        }
        _ => Err(type_error("name expects URLQueryItem")),
    }
}

fn url_query_item_value_prop(recv: SwiftValue) -> StdResult {
    match &recv {
        SwiftValue::Struct(o) if o.type_name == "URLQueryItem" => {
            Ok(o.get("value").cloned().unwrap_or(SwiftValue::Nil))
        }
        _ => Err(type_error("value expects URLQueryItem")),
    }
}

fn url_query_item_description(recv: SwiftValue) -> StdResult {
    let SwiftValue::Struct(o) = &recv else {
        return Err(type_error("description expects URLQueryItem"));
    };
    let name = match o.get("name") {
        Some(SwiftValue::Str(s)) => s.to_string(),
        _ => String::new(),
    };
    let value = match o.get("value") {
        Some(SwiftValue::Str(s)) => s.to_string(),
        _ => String::new(),
    };
    Ok(SwiftValue::Str(format!("{name}={value}").into()))
}

fn url_components_description(recv: SwiftValue) -> StdResult {
    // The reconstructed URL string, or empty when components are insufficient.
    match url_components_string(recv)? {
        SwiftValue::Str(s) => Ok(SwiftValue::Str(s)),
        _ => Ok(SwiftValue::Str(String::new().into())),
    }
}

// ---------------------------------------------------------------------------
// Percent-encoded component accessors
// ---------------------------------------------------------------------------

/// Helper: return `percent_encode_path` of the stored (decoded) path.
fn url_components_percent_encoded_path(recv: SwiftValue) -> StdResult {
    let path = comp_str(&recv, "path").unwrap_or_default();
    Ok(SwiftValue::Str(percent_encode_path(&path)))
}

/// Helper: encode the stored (decoded) optional string with `f`, or return Nil.
fn encode_opt_comp(recv: &SwiftValue, field: &str, f: impl Fn(&str) -> String) -> StdResult {
    match comp_str(recv, field) {
        Some(s) => Ok(SwiftValue::Str(f(&s))),
        None => Ok(SwiftValue::Nil),
    }
}

fn url_components_percent_encoded_query(recv: SwiftValue) -> StdResult {
    encode_opt_comp(&recv, "query", percent_encode_query)
}
fn url_components_percent_encoded_fragment(recv: SwiftValue) -> StdResult {
    encode_opt_comp(&recv, "fragment", percent_encode_query)
}
fn url_components_percent_encoded_user(recv: SwiftValue) -> StdResult {
    encode_opt_comp(&recv, "user", percent_encode_userinfo)
}
fn url_components_percent_encoded_password(recv: SwiftValue) -> StdResult {
    encode_opt_comp(&recv, "password", percent_encode_userinfo)
}
/// `percentEncodedHost` getter.
///
/// When `encodedHost` was used to set a verbatim encoded form, that form is
/// stored in the private `"_encodedHost"` field and returned here.  Otherwise
/// the canonical (decoded) host is returned as-is — for ASCII-only hosts
/// percent-encoding/decoding is a no-op, so the canonical form equals the
/// percent-encoded form.
fn url_components_percent_encoded_host(recv: SwiftValue) -> StdResult {
    // Prefer the verbatim stored form when an encodedHost override is present
    // (stored as SwiftValue::Str). A Nil entry means the override was cleared.
    if let Some(SwiftValue::Str(s)) = comp_field(&recv, "_encodedHost") {
        return Ok(SwiftValue::Str(s.clone()));
    }
    Ok(comp_field(&recv, "host")
        .cloned()
        .unwrap_or(SwiftValue::Nil))
}

/// `encodedHost` getter — same storage as `percentEncodedHost`.
fn url_components_encoded_host(recv: SwiftValue) -> StdResult {
    url_components_percent_encoded_host(recv)
}

/// `percentEncodedQueryItems` — parse the stored decoded query, then
/// percent-encode each name and value with the query-item charset.
fn url_components_percent_encoded_query_items(recv: SwiftValue) -> StdResult {
    let Some(query) = comp_str(&recv, "query") else {
        return Ok(SwiftValue::Nil);
    };
    if query.is_empty() {
        return Ok(SwiftValue::Nil);
    }
    // NOTE: splitting on '&' is correct for the common case but fails when
    // the stored (decoded) query has a literal '&' inside a name or value
    // (which would have been %26-encoded in the original URL). This is a
    // known limitation documented in notes.md.
    let items: Vec<SwiftValue> = query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => url_query_item_value(
                percent_encode_query_item(k),
                Some(percent_encode_query_item(v)),
            ),
            None => url_query_item_value(percent_encode_query_item(pair), None),
        })
        .collect();
    Ok(SwiftValue::Array(Rc::new(items)))
}

// ---------------------------------------------------------------------------
// Percent-encoded component setters
// ---------------------------------------------------------------------------

/// Emit the Foundation-style fatal-error for an illegally encoded component.
#[inline]
fn invalid_chars_trap(prop: &str) -> StdError {
    StdError::Error(tswift_core::EvalError::Trap(format!(
        "Attempting to set {prop} with invalid characters"
    )))
}

/// Validate that `s` is a legally percent-encoded string for the given
/// component.  A byte is accepted when it either:
/// - is a `%` followed immediately by exactly two ASCII hex digits, OR
/// - passes `is_allowed(byte)` — i.e. the byte belongs to the RFC 3986
///   character set for that component (as implemented by Foundation's
///   `CharacterSet.url*Allowed`).
///
/// Anything else — including an unescaped space, `#`, or any control byte —
/// triggers a `fatalError` trap matching Foundation's message.
fn validate_encoded_component(
    s: &str,
    prop: &str,
    is_allowed: fn(u8) -> bool,
) -> Result<(), StdError> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'%' {
            let hi = bytes.get(i + 1).copied().map(|b| (b as char).to_digit(16));
            let lo = bytes.get(i + 2).copied().map(|b| (b as char).to_digit(16));
            if !matches!((hi, lo), (Some(Some(_)), Some(Some(_)))) {
                return Err(invalid_chars_trap(prop));
            }
            i += 3;
        } else if is_allowed(b) {
            i += 1;
        } else {
            return Err(invalid_chars_trap(prop));
        }
    }
    Ok(())
}

// ---- Per-component allowed-byte predicates (mirror Foundation CharacterSet) ---
//
// Ground-truthed against `CharacterSet.url*Allowed` on macOS Swift 6.3.2:
//   path:     !$&'()*+,-./0-9:;=@A-Z_a-z~
//   query:    !$&'()*+,-./0-9:;=?@A-Z_a-z~   (adds '?')
//   fragment: same as query
//   user:     !$&'()*+,-.0-9;=A-Z_a-z~       (no '/', ':', '@')
//   password: same as user
//   host:     !$&'()*+,-.0-9:;=A-Z[]_a-z~   (no '/', '?', '@'; adds '[', ']', ':')

#[inline]
fn is_allowed_path(b: u8) -> bool {
    matches!(b,
        b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b','
        | b'-' | b'.' | b'/'
        | b'0'..=b'9'
        | b':' | b';' | b'='
        | b'@'
        | b'A'..=b'Z'
        | b'_'
        | b'a'..=b'z'
        | b'~'
    )
}

#[inline]
fn is_allowed_query(b: u8) -> bool {
    // query adds '?' to path
    b == b'?' || is_allowed_path(b)
}

// fragment uses the same set as query
#[inline]
fn is_allowed_fragment(b: u8) -> bool {
    is_allowed_query(b)
}

#[inline]
fn is_allowed_userinfo(b: u8) -> bool {
    // user/password: no '/', ':', '@'
    matches!(b,
        b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b','
        | b'-' | b'.'
        | b'0'..=b'9'
        | b';' | b'='
        | b'A'..=b'Z'
        | b'_'
        | b'a'..=b'z'
        | b'~'
    )
}

#[inline]
fn is_allowed_host(b: u8) -> bool {
    // host: no '/', '?', '@'; adds '[', ']', ':'
    matches!(b,
        b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b','
        | b'-' | b'.'
        | b'0'..=b'9'
        | b':' | b';' | b'='
        | b'A'..=b'Z'
        | b'[' | b']'
        | b'_'
        | b'a'..=b'z'
        | b'~'
    )
}

/// Query-item **name** charset: `urlQueryAllowed` minus `&` and `=`.
#[inline]
fn is_allowed_query_item_name(b: u8) -> bool {
    // Exclude '&' (item separator) and '=' (key=value separator) from query set
    !matches!(b, b'&' | b'=') && is_allowed_query(b)
}

/// Query-item **value** charset: full `urlQueryAllowed` (Foundation permits
/// unescaped `&` and `=` in values — the caller is trusted to encode if needed).
#[inline]
fn is_allowed_query_item_value(b: u8) -> bool {
    is_allowed_query(b)
}

/// Helper: build an updated `URLComponents` struct with `field` set to `value`.
fn comp_set_field(
    recv: SwiftValue,
    field: &str,
    value: SwiftValue,
) -> Result<SwiftValue, StdError> {
    let SwiftValue::Struct(obj) = recv else {
        return Err(crate::type_error("expected URLComponents"));
    };
    let mut obj = (*obj).clone();
    obj.set(field, value);
    Ok(SwiftValue::Struct(Rc::new(obj)))
}

/// Setter for `percentEncodedPath`: validate against path allowed set, then
/// store the DECODED path in the `"path"` field.
fn set_percent_encoded_path(
    recv: SwiftValue,
    new_value: SwiftValue,
) -> Result<SwiftValue, StdError> {
    let SwiftValue::Str(s) = &new_value else {
        return Err(crate::type_error("percentEncodedPath must be a String"));
    };
    validate_encoded_component(s, "percentEncodedPath", is_allowed_path)?;
    comp_set_field(recv, "path", SwiftValue::Str(percent_decode(s)))
}

/// Setter for `percentEncodedQuery`.
fn set_percent_encoded_query(
    recv: SwiftValue,
    new_value: SwiftValue,
) -> Result<SwiftValue, StdError> {
    match &new_value {
        SwiftValue::Nil => comp_set_field(recv, "query", SwiftValue::Nil),
        SwiftValue::Str(s) => {
            validate_encoded_component(s, "percentEncodedQuery", is_allowed_query)?;
            comp_set_field(recv, "query", SwiftValue::Str(percent_decode(s)))
        }
        _ => Err(crate::type_error("percentEncodedQuery must be String?")),
    }
}

/// Setter for `percentEncodedFragment`.
fn set_percent_encoded_fragment(
    recv: SwiftValue,
    new_value: SwiftValue,
) -> Result<SwiftValue, StdError> {
    match &new_value {
        SwiftValue::Nil => comp_set_field(recv, "fragment", SwiftValue::Nil),
        SwiftValue::Str(s) => {
            validate_encoded_component(s, "percentEncodedFragment", is_allowed_fragment)?;
            comp_set_field(recv, "fragment", SwiftValue::Str(percent_decode(s)))
        }
        _ => Err(crate::type_error("percentEncodedFragment must be String?")),
    }
}

/// Setter for `percentEncodedUser`.
fn set_percent_encoded_user(
    recv: SwiftValue,
    new_value: SwiftValue,
) -> Result<SwiftValue, StdError> {
    match &new_value {
        SwiftValue::Nil => comp_set_field(recv, "user", SwiftValue::Nil),
        SwiftValue::Str(s) => {
            validate_encoded_component(s, "percentEncodedUser", is_allowed_userinfo)?;
            comp_set_field(recv, "user", SwiftValue::Str(percent_decode(s)))
        }
        _ => Err(crate::type_error("percentEncodedUser must be String?")),
    }
}

/// Setter for `percentEncodedPassword`.
fn set_percent_encoded_password(
    recv: SwiftValue,
    new_value: SwiftValue,
) -> Result<SwiftValue, StdError> {
    match &new_value {
        SwiftValue::Nil => comp_set_field(recv, "password", SwiftValue::Nil),
        SwiftValue::Str(s) => {
            validate_encoded_component(s, "percentEncodedPassword", is_allowed_userinfo)?;
            comp_set_field(recv, "password", SwiftValue::Str(percent_decode(s)))
        }
        _ => Err(crate::type_error("percentEncodedPassword must be String?")),
    }
}

/// Helper: set `"host"` to `host_val` and clear the `"_encodedHost"` override.
fn comp_set_host_canonical(recv: SwiftValue, host_val: SwiftValue) -> Result<SwiftValue, StdError> {
    let SwiftValue::Struct(obj) = recv else {
        return Err(crate::type_error("expected URLComponents"));
    };
    let mut obj = (*obj).clone();
    obj.set("host", host_val);
    // Clear any verbatim encodedHost override so getters return the
    // canonical form from now on.
    obj.set("_encodedHost", SwiftValue::Nil);
    Ok(SwiftValue::Struct(Rc::new(obj)))
}

/// Setter for `host` (plain decoded).
///
/// Registered so that `c.host = "…"` also clears the `_encodedHost` override
/// that may have been set by `encodedHost`.
fn set_host(recv: SwiftValue, new_value: SwiftValue) -> Result<SwiftValue, StdError> {
    match new_value {
        SwiftValue::Nil => comp_set_host_canonical(recv, SwiftValue::Nil),
        SwiftValue::Str(_) => comp_set_host_canonical(recv, new_value),
        _ => Err(crate::type_error("host must be String?")),
    }
}

/// Setter for `percentEncodedHost`.
///
/// Foundation decodes the input and stores the canonical (decoded) form;
/// both `host` and `percentEncodedHost` then return the decoded string.
/// The `_encodedHost` override is cleared (no verbatim storage).
fn set_percent_encoded_host(
    recv: SwiftValue,
    new_value: SwiftValue,
) -> Result<SwiftValue, StdError> {
    match &new_value {
        SwiftValue::Nil => comp_set_host_canonical(recv, SwiftValue::Nil),
        SwiftValue::Str(s) => {
            validate_encoded_component(s, "percentEncodedHost", is_allowed_host)?;
            // Decode and store canonical; clear verbatim override.
            let decoded = percent_decode(s);
            comp_set_host_canonical(recv, SwiftValue::Str(decoded))
        }
        _ => Err(crate::type_error("percentEncodedHost must be String?")),
    }
}

/// Setter for `encodedHost`.
///
/// Stores the verbatim encoded string in `_encodedHost` (returned by
/// `encodedHost` and `percentEncodedHost` getters) while the decoded form
/// goes into `host`.  Assigning `host` or `percentEncodedHost` later clears
/// the override and reverts to canonical behaviour.
fn set_encoded_host(recv: SwiftValue, new_value: SwiftValue) -> Result<SwiftValue, StdError> {
    match &new_value {
        SwiftValue::Nil => comp_set_host_canonical(recv, SwiftValue::Nil),
        SwiftValue::Str(s) => {
            validate_encoded_component(s, "encodedHost", is_allowed_host)?;
            let verbatim = s.clone();
            let decoded = percent_decode(s);
            // Store decoded host + verbatim override.
            let SwiftValue::Struct(obj) = recv else {
                return Err(crate::type_error("expected URLComponents"));
            };
            let mut obj = (*obj).clone();
            obj.set("host", SwiftValue::Str(decoded));
            obj.set("_encodedHost", SwiftValue::Str(verbatim));
            Ok(SwiftValue::Struct(Rc::new(obj)))
        }
        _ => Err(crate::type_error("encodedHost must be String?")),
    }
}

/// Setter for `percentEncodedQueryItems`.
/// Each item's name and value are treated as already-encoded; they are decoded
/// and assembled into the canonical `query` field.
///
/// Validation:
/// - Name: `urlQueryAllowed` minus `&` and `=` (structural separators).
/// - Value: full `urlQueryAllowed` (Foundation permits unescaped `&`/`=` in
///   values; the caller is responsible for encoding them if needed).
fn set_percent_encoded_query_items(
    recv: SwiftValue,
    new_value: SwiftValue,
) -> Result<SwiftValue, StdError> {
    match new_value {
        SwiftValue::Nil => comp_set_field(recv, "query", SwiftValue::Nil),
        SwiftValue::Array(items) => {
            let mut parts: Vec<String> = Vec::with_capacity(items.len());
            for item in items.iter() {
                let SwiftValue::Struct(obj) = item else {
                    return Err(crate::type_error(
                        "percentEncodedQueryItems items must be URLQueryItem",
                    ));
                };
                let name_enc = match obj.get("name") {
                    Some(SwiftValue::Str(s)) => s.clone(),
                    _ => String::new(),
                };
                validate_encoded_component(
                    &name_enc,
                    "percentEncodedQueryItems",
                    is_allowed_query_item_name,
                )?;
                let decoded_name = percent_decode(&name_enc);
                match obj.get("value") {
                    Some(SwiftValue::Str(v)) => {
                        validate_encoded_component(
                            v,
                            "percentEncodedQueryItems",
                            is_allowed_query_item_value,
                        )?;
                        let decoded_value = percent_decode(v);
                        parts.push(format!("{decoded_name}={decoded_value}"));
                    }
                    Some(SwiftValue::Nil) | None => {
                        parts.push(decoded_name);
                    }
                    _ => {
                        return Err(crate::type_error("URLQueryItem value must be String?"));
                    }
                }
            }
            let query = if parts.is_empty() {
                SwiftValue::Nil
            } else {
                SwiftValue::Str(parts.join("&"))
            };
            comp_set_field(recv, "query", query)
        }
        _ => Err(crate::type_error(
            "percentEncodedQueryItems must be [URLQueryItem]?",
        )),
    }
}

/// Setter for `queryItems`: converts `[URLQueryItem]?` into the canonical
/// `query` string and stores it in the `"query"` field.
///
/// Setting `queryItems` to `[]` stores an empty query string `""` (which
/// produces a `?` in the URL with no params), matching Foundation behavior.
/// Setting to `nil` clears the query.
fn set_query_items(recv: SwiftValue, new_value: SwiftValue) -> Result<SwiftValue, StdError> {
    match new_value {
        SwiftValue::Nil => comp_set_field(recv, "query", SwiftValue::Nil),
        SwiftValue::Array(items) => {
            let mut parts: Vec<String> = Vec::with_capacity(items.len());
            for item in items.iter() {
                let SwiftValue::Struct(obj) = item else {
                    return Err(crate::type_error(
                        "queryItems elements must be URLQueryItem",
                    ));
                };
                let name = match obj.get("name") {
                    Some(SwiftValue::Str(s)) => s.clone(),
                    _ => String::new(),
                };
                match obj.get("value") {
                    Some(SwiftValue::Str(v)) => parts.push(format!("{name}={v}")),
                    Some(SwiftValue::Nil) | None => parts.push(name),
                    _ => return Err(crate::type_error("URLQueryItem value must be String?")),
                }
            }
            // Empty array → empty query string (produces `?` in URL).
            let query = parts.join("&");
            comp_set_field(recv, "query", SwiftValue::Str(query))
        }
        _ => Err(crate::type_error("queryItems must be [URLQueryItem]?")),
    }
}

const URL_COMPONENTS_ENCODED_SETTERS: &[(&str, PropertySetterFn)] = &[
    // `queryItems` setter: translates item array to the `query` string field.
    ("queryItems", set_query_items),
    // Plain field setters that need side-effects (clearing _encodedHost).
    ("host", set_host),
    // Percent-encoded setters.
    ("percentEncodedPath", set_percent_encoded_path),
    ("percentEncodedQuery", set_percent_encoded_query),
    ("percentEncodedFragment", set_percent_encoded_fragment),
    ("percentEncodedUser", set_percent_encoded_user),
    ("percentEncodedPassword", set_percent_encoded_password),
    ("percentEncodedHost", set_percent_encoded_host),
    ("encodedHost", set_encoded_host),
    ("percentEncodedQueryItems", set_percent_encoded_query_items),
];

const URL_COMPONENTS_ENCODED_GETTERS: &[(&str, tswift_core::PropertyFn)] = &[
    ("percentEncodedPath", url_components_percent_encoded_path),
    ("percentEncodedQuery", url_components_percent_encoded_query),
    (
        "percentEncodedFragment",
        url_components_percent_encoded_fragment,
    ),
    ("percentEncodedUser", url_components_percent_encoded_user),
    (
        "percentEncodedPassword",
        url_components_percent_encoded_password,
    ),
    ("percentEncodedHost", url_components_percent_encoded_host),
    ("encodedHost", url_components_encoded_host),
    (
        "percentEncodedQueryItems",
        url_components_percent_encoded_query_items,
    ),
];

// ===========================================================================
// URL value model
// ===========================================================================

#[derive(Default, Clone)]
struct Parsed {
    scheme: Option<String>,
    /// Whether an authority component (`//...`) was present, even if empty —
    /// needed to round-trip `file:///path` (empty authority) faithfully.
    authority: bool,
    user: Option<String>,
    password: Option<String>,
    host: Option<String>,
    port: Option<i128>,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
}

/// Parse an absolute URL string into its components (best-effort RFC 3986).
fn parse_url(input: &str) -> Parsed {
    let mut p = Parsed::default();
    let mut rest = input;

    // fragment
    if let Some(idx) = rest.find('#') {
        p.fragment = Some(rest[idx + 1..].to_string());
        rest = &rest[..idx];
    }
    // query
    if let Some(idx) = rest.find('?') {
        p.query = Some(rest[idx + 1..].to_string());
        rest = &rest[..idx];
    }
    // scheme
    if let Some(idx) = rest.find(':') {
        let candidate = &rest[..idx];
        let is_scheme = !candidate.is_empty()
            && candidate
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
            && candidate
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
        if is_scheme {
            p.scheme = Some(candidate.to_string());
            rest = &rest[idx + 1..];
        }
    }
    // authority
    if let Some(after) = rest.strip_prefix("//") {
        p.authority = true;
        let auth_end = after.find('/').unwrap_or(after.len());
        let authority = &after[..auth_end];
        rest = &after[auth_end..];
        let host_port = if let Some(at) = authority.rfind('@') {
            let userinfo = &authority[..at];
            if let Some(colon) = userinfo.find(':') {
                p.user = Some(userinfo[..colon].to_string());
                p.password = Some(userinfo[colon + 1..].to_string());
            } else if !userinfo.is_empty() {
                p.user = Some(userinfo.to_string());
            }
            &authority[at + 1..]
        } else {
            authority
        };
        // A bracketed IPv6 literal (`[::1]`) keeps its brackets as the host; a
        // port, if any, follows the closing `]`.
        let (host, port_text) = if let Some(close) = host_port
            .strip_prefix('[')
            .and_then(|_| host_port.find(']'))
        {
            let host = &host_port[..=close];
            let after = &host_port[close + 1..];
            (host, after.strip_prefix(':'))
        } else if let Some(colon) = host_port.rfind(':') {
            (&host_port[..colon], Some(&host_port[colon + 1..]))
        } else {
            (host_port, None)
        };
        if !host.is_empty() {
            p.host = Some(host.to_string());
        }
        // A port must be a non-empty run of ASCII digits.
        if let Some(text) = port_text {
            if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
                if let Ok(port) = text.parse::<i128>() {
                    p.port = Some(port);
                }
            }
        }
    }
    p.path = rest.to_string();
    p
}

pub(crate) fn url_value(string: String) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "URL".into(),
        fields: vec![("_string".into(), SwiftValue::Str(string))],
    }))
}

pub(crate) fn url_string(value: &SwiftValue) -> Result<String, StdError> {
    let SwiftValue::Struct(obj) = value else {
        return Err(type_error(format!(
            "expected URL, got {}",
            value.type_name()
        )));
    };
    if obj.type_name != "URL" {
        return Err(type_error(format!("expected URL, got {}", obj.type_name)));
    }
    match obj.get("_string") {
        Some(SwiftValue::Str(s)) => Ok(s.clone()),
        _ => Err(type_error("malformed URL value")),
    }
}

/// A URL is a file URL when its scheme is `file` (case-insensitive), matching
/// Foundation's semantic `isFileURL` rather than initializer provenance.
fn url_is_file_flag(value: &SwiftValue) -> bool {
    url_string(value)
        .ok()
        .and_then(|s| parse_url(&s).scheme)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("file"))
}

fn opt_str(v: Option<String>) -> SwiftValue {
    v.map(SwiftValue::Str).unwrap_or(SwiftValue::Nil)
}

// ---- URL init -------------------------------------------------------------

fn url_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if args.len() != 1 {
        return Err(type_error("URL expects one argument"));
    }
    let SwiftValue::Str(raw) = &args[0].value else {
        return Err(type_error("URL argument must be a String"));
    };
    match args[0].label.as_deref() {
        // `URL(string:)` is failable: returns `nil` for strings that cannot
        // be a valid URL (empty or containing unencoded whitespace).  This
        // validation is shared with the JSON URL decoder via
        // `tswift_core::is_url_string_valid`.
        Some("string") => Ok(if tswift_core::is_url_string_valid(raw) {
            url_value(raw.clone())
        } else {
            SwiftValue::Nil
        }),
        Some("fileURLWithPath") => {
            let path = raw.clone();
            let string = format!(
                "file://{}",
                if path.starts_with('/') {
                    path
                } else {
                    format!("/{path}")
                }
            );
            Ok(url_value(string))
        }
        Some(other) => Err(type_error(format!("unsupported URL argument {other}:"))),
        None => Err(type_error("URL argument needs a label")),
    }
}

// ---- URL accessors --------------------------------------------------------

fn url_absolute_string(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Str(url_string(&recv)?))
}

fn url_scheme(recv: SwiftValue) -> StdResult {
    Ok(opt_str(parse_url(&url_string(&recv)?).scheme))
}
fn url_host(recv: SwiftValue) -> StdResult {
    Ok(opt_str(parse_url(&url_string(&recv)?).host))
}
fn url_port(recv: SwiftValue) -> StdResult {
    Ok(parse_url(&url_string(&recv)?)
        .port
        .map(SwiftValue::int)
        .unwrap_or(SwiftValue::Nil))
}
fn url_path(recv: SwiftValue) -> StdResult {
    // Foundation's `URL.path` is percent-decoded.
    Ok(SwiftValue::Str(percent_decode(
        &parse_url(&url_string(&recv)?).path,
    )))
}
fn url_base_url(_recv: SwiftValue) -> StdResult {
    // No base URL is modelled; URLs are always absolute here.
    Ok(SwiftValue::Nil)
}
fn url_has_directory_path(recv: SwiftValue) -> StdResult {
    let path = parse_url(&url_string(&recv)?).path;
    Ok(SwiftValue::Bool(path.ends_with('/')))
}
fn url_query(recv: SwiftValue) -> StdResult {
    Ok(opt_str(parse_url(&url_string(&recv)?).query))
}
fn url_fragment(recv: SwiftValue) -> StdResult {
    Ok(opt_str(parse_url(&url_string(&recv)?).fragment))
}
fn url_user(recv: SwiftValue) -> StdResult {
    Ok(opt_str(parse_url(&url_string(&recv)?).user))
}
fn url_password(recv: SwiftValue) -> StdResult {
    Ok(opt_str(parse_url(&url_string(&recv)?).password))
}
fn url_is_file(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Bool(url_is_file_flag(&recv)))
}

fn url_last_path_component(recv: SwiftValue) -> StdResult {
    let path = parse_url(&url_string(&recv)?).path;
    Ok(SwiftValue::Str(last_component(&path)))
}

fn url_path_extension(recv: SwiftValue) -> StdResult {
    let last = last_component(&parse_url(&url_string(&recv)?).path);
    let ext = file_extension(&last)
        .map(|(_, e)| e.to_string())
        .unwrap_or_default();
    Ok(SwiftValue::Str(ext))
}

/// Split a path component into `(stem, extension)` when it has a file
/// extension. A leading-dot hidden file (`.bashrc`) or trailing dot has no
/// extension, matching Foundation/`NSString` semantics.
fn file_extension(name: &str) -> Option<(&str, &str)> {
    let dot = name.rfind('.')?;
    let (stem, ext) = (&name[..dot], &name[dot + 1..]);
    if stem.is_empty() || ext.is_empty() {
        None
    } else {
        Some((stem, ext))
    }
}

/// Decode `%XX` escapes in a URL component (best-effort; invalid escapes are
/// left intact).
pub(crate) fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn url_path_components(recv: SwiftValue) -> StdResult {
    let path = parse_url(&url_string(&recv)?).path;
    let mut comps: Vec<SwiftValue> = Vec::new();
    if path.starts_with('/') {
        comps.push(SwiftValue::Str("/".into()));
    }
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        comps.push(SwiftValue::Str(seg.to_string()));
    }
    Ok(SwiftValue::Array(Rc::new(comps)))
}

/// The last non-empty path segment, or `/` for a root path.
fn last_component(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rsplit('/').next() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ if path.starts_with('/') => "/".to_string(),
        _ => String::new(),
    }
}

// ---- URL path manipulation ------------------------------------------------

/// Rebuild a URL string from a parsed value with a replaced path.
fn rebuild_with_path(parsed: &Parsed, new_path: &str) -> String {
    let mut out = String::new();
    if let Some(scheme) = &parsed.scheme {
        out.push_str(scheme);
        out.push(':');
    }
    if parsed.authority || parsed.host.is_some() || parsed.user.is_some() {
        out.push_str("//");
        if let Some(user) = &parsed.user {
            out.push_str(user);
            if let Some(pw) = &parsed.password {
                out.push(':');
                out.push_str(pw);
            }
            out.push('@');
        }
        if let Some(host) = &parsed.host {
            out.push_str(host);
        }
        if let Some(port) = parsed.port {
            out.push(':');
            out.push_str(&port.to_string());
        }
    }
    out.push_str(new_path);
    if let Some(query) = &parsed.query {
        out.push('?');
        out.push_str(query);
    }
    if let Some(frag) = &parsed.fragment {
        out.push('#');
        out.push_str(frag);
    }
    out
}

fn join_path(base: &str, component: &str) -> String {
    if base.is_empty() {
        component.to_string()
    } else if base.ends_with('/') {
        format!("{base}{component}")
    } else {
        format!("{base}/{component}")
    }
}

fn appended_component(recv: &SwiftValue, args: &[SwiftValue]) -> Result<SwiftValue, StdError> {
    let [SwiftValue::Str(comp)] = args else {
        return Err(type_error("URL.appendingPathComponent expects one String"));
    };
    let parsed = parse_url(&url_string(recv)?);
    // Foundation's `appendingPathComponent` percent-encodes the component
    // (same charset as `appending(component:)`), so spaces and other special
    // chars are escaped.  An empty component appends a trailing slash.
    let encoded = percent_encode_component(comp);
    let new_path = if encoded.is_empty() {
        format!("{}/", parsed.path.trim_end_matches('/'))
    } else {
        join_path(&parsed.path, &encoded)
    };
    Ok(url_value(rebuild_with_path(&parsed, &new_path)))
}

fn url_appending_path_component(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let result = appended_component(&recv, &args)?;
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

fn url_append_path_component(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let updated = appended_component(&recv, &args)?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: updated,
    })
}

fn deleted_last(recv: &SwiftValue) -> Result<SwiftValue, StdError> {
    let parsed = parse_url(&url_string(recv)?);
    let trimmed = parsed.path.trim_end_matches('/');
    let new_path = match trimmed.rsplit_once('/') {
        Some(("", _)) => "/".to_string(),
        Some((head, _)) => format!("{head}/"),
        None => String::new(),
    };
    Ok(url_value(rebuild_with_path(&parsed, &new_path)))
}

fn url_deleting_last_path_component(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let result = deleted_last(&recv)?;
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

fn url_delete_last_path_component(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let updated = deleted_last(&recv)?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: updated,
    })
}

fn url_appending_path_extension(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let [SwiftValue::Str(ext)] = args.as_slice() else {
        return Err(type_error("URL.appendingPathExtension expects one String"));
    };
    let parsed = parse_url(&url_string(&recv)?);
    // Foundation preserves a trailing slash: "/a/b/" + ext "txt" -> "/a/b.txt/".
    let had_slash = parsed.path.ends_with('/');
    let stem = parsed.path.trim_end_matches('/');
    let new_path = if had_slash {
        format!("{stem}.{ext}/")
    } else {
        format!("{stem}.{ext}")
    };
    let result = url_value(rebuild_with_path(&parsed, &new_path));
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

fn url_deleting_path_extension(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let parsed = parse_url(&url_string(&recv)?);
    let path = &parsed.path;
    // Strip the extension from the final component only.
    let new_path = match path.rsplit_once('/') {
        Some((head, last)) => match file_extension(last) {
            Some((stem, _)) => format!("{head}/{stem}"),
            None => path.clone(),
        },
        None => match file_extension(path) {
            Some((stem, _)) => stem.to_string(),
            None => path.clone(),
        },
    };
    let result = url_value(rebuild_with_path(&parsed, &new_path));
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

// ---- Path manipulation helpers -------------------------------------------

/// Percent-encode a query string or fragment (RFC 3986 query charset).
/// Allowed: unreserved, sub-delims, ':', '@', '/', '?', '='
/// Encodes: space, '"', '#', '%', '<', '>', '[', '\\', ']', '^', '`', '{', '|', '}'
/// (Matches Foundation's percentEncodedQuery/percentEncodedFragment behaviour.)
fn percent_encode_query(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    for b in input.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b'!'
            | b'$'
            | b'&'
            | b'\'' // apostrophe
            | b'('
            | b')'
            | b'*'
            | b'+'
            | b','
            | b';'
            | b':'
            | b'@'
            | b'/'
            | b'?'
            | b'=' => out.push(b as char),
            other => {
                out.push('%');
                out.push(
                    char::from_digit((other >> 4) as u32, 16)
                        .unwrap()
                        .to_ascii_uppercase(),
                );
                out.push(
                    char::from_digit((other & 0xf) as u32, 16)
                        .unwrap()
                        .to_ascii_uppercase(),
                );
            }
        }
    }
    out
}

/// Percent-encode a userinfo component (user or password).
/// Allowed: unreserved, sub-delims (including '&' and '='), ':'
/// Does NOT allow '/', '?', '@' (which query allows).
fn percent_encode_userinfo(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    for b in input.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b'!'
            | b'$'
            | b'&'
            | b'\'' // apostrophe
            | b'('
            | b')'
            | b'*'
            | b'+'
            | b','
            | b';'
            | b'='
            | b':' => out.push(b as char),
            other => {
                out.push('%');
                out.push(
                    char::from_digit((other >> 4) as u32, 16)
                        .unwrap()
                        .to_ascii_uppercase(),
                );
                out.push(
                    char::from_digit((other & 0xf) as u32, 16)
                        .unwrap()
                        .to_ascii_uppercase(),
                );
            }
        }
    }
    out
}

/// Percent-encode a single query-item name or value.
/// Same as query charset but additionally encodes '&' and '='.
fn percent_encode_query_item(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    for b in input.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b'!'
            | b'$'
            | b'\'' // apostrophe
            | b'('
            | b')'
            | b'*'
            | b'+'
            | b','
            | b';'
            | b':'
            | b'@'
            | b'/'
            | b'?' => out.push(b as char),
            other => {
                out.push('%');
                out.push(
                    char::from_digit((other >> 4) as u32, 16)
                        .unwrap()
                        .to_ascii_uppercase(),
                );
                out.push(
                    char::from_digit((other & 0xf) as u32, 16)
                        .unwrap()
                        .to_ascii_uppercase(),
                );
            }
        }
    }
    out
}

/// Percent-encode characters that must be encoded in a URL path segment.
/// Encodes everything except unreserved characters and `/`.
fn percent_encode_path(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        match b {
            // unreserved
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            // sub-delims allowed in path
            | b'!'
            | b'$'
            | b'&'
            | b'\'' // apostrophe
            | b'('
            | b')'
            | b'*'
            | b'+'
            | b','
            | b';'
            | b'='
            // path separator
            | b'/'
            // pchar extras
            | b':'
            | b'@' => out.push(b as char),
            other => {
                out.push('%');
                out.push(char::from_digit((other >> 4) as u32, 16).unwrap().to_ascii_uppercase());
                out.push(char::from_digit((other & 0xf) as u32, 16).unwrap().to_ascii_uppercase());
            }
        }
    }
    out
}

/// Percent-encode a single path component (no `/` allowed through).
fn percent_encode_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b'!'
            | b'$'
            | b'&'
            | b'\'' // apostrophe
            | b'('
            | b')'
            | b'*'
            | b'+'
            | b','
            | b';'
            | b'='
            | b':'
            | b'@' => out.push(b as char),
            other => {
                out.push('%');
                out.push(char::from_digit((other >> 4) as u32, 16).unwrap().to_ascii_uppercase());
                out.push(char::from_digit((other & 0xf) as u32, 16).unwrap().to_ascii_uppercase());
            }
        }
    }
    out
}

/// Resolve `.` and `..` segments lexically, matching Foundation `standardized`.
///
/// Rules:
/// - `.` segments are dropped.
/// - `..` pops the last **non-empty** segment (so double-slash runs are
///   skipped over when searching for what to pop).
/// - For absolute paths (leading `/`) `..` is clamped at the root: if there
///   is no non-empty segment to pop the `..` is silently discarded.
/// - For relative paths a `..` that cannot be resolved is kept as-is.
/// - Double slashes are **preserved** — Foundation `standardized` does not
///   collapse `//`; only `.`/`..` are affected.
fn standardize_path(path: &str) -> String {
    let is_absolute = path.starts_with('/');
    // Collect all raw split segments (empty strings represent the gaps that
    // produce double slashes on join).
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "." => {} // self-reference: discard
            ".." => {
                // Foundation's `standardized` resolves `..` by popping the
                // most-recent segment — whether empty or not — so that `..`
                // after a double-slash (empty segment) steps over the gap.
                // For absolute paths clamp at the root marker (the leading
                // `""` produced by the initial `/`); for relative paths keep
                // the unresolvable `..`.
                let min_len = if is_absolute { 1 } else { 0 };
                if out.len() > min_len {
                    out.pop();
                } else if !is_absolute {
                    // Relative path with nothing to resolve: keep the `..`.
                    out.push("..");
                }
                // Absolute at root: clamp — discard the `..`.
            }
            other => out.push(other),
        }
    }
    let result = out.join("/");
    // For absolute paths whose every segment was discarded (e.g. `/.` or
    // `/..`), `out` ends up as `[""]` whose join is `""`.  The minimum
    // canonical absolute path is `/`.
    if is_absolute && result.is_empty() {
        "/".to_string()
    } else {
        result
    }
}

// ---- appending(path:) / appending(component:) ----------------------------

/// Helper: apply `appending(path:)` or `appending(component:)` semantics.
///
/// - `path:` label: the argument is a possibly-multi-segment path string.
///   A leading `/` does **not** replace the existing path — it is stripped
///   and the remainder is joined.  An empty argument (or all-slashes)
///   appends a trailing slash per Foundation semantics.
/// - `component:` label: the argument is a single opaque component;
///   all characters including `/` are percent-encoded and the result is
///   joined.
fn do_appending(recv: &SwiftValue, arg: &Arg) -> Result<SwiftValue, StdError> {
    let SwiftValue::Str(val) = &arg.value else {
        return Err(type_error(
            "URL.appending(path:/component:) expects a String",
        ));
    };
    let parsed = parse_url(&url_string(recv)?);
    let new_path = match arg.label.as_deref() {
        Some("component") => {
            // Single component — percent-encode everything including `/`, always join.
            join_path(&parsed.path, &percent_encode_component(val))
        }
        // `path:` or unrecognised label — treat as multi-segment path.
        _ => {
            // Strip exactly ONE leading slash: Foundation `appending(path:)`
            // does not let a leading '/' replace the existing path, but a
            // second slash (e.g. `//other`) must be preserved so that
            // `appending(path: "//other")` on `/dir` yields `/dir//other`.
            let stripped = val.strip_prefix('/').unwrap_or(val);
            if stripped.is_empty() {
                // Empty arg (or a single '/') → append trailing slash.
                if parsed.path.ends_with('/') {
                    parsed.path.clone()
                } else {
                    format!("{}/", parsed.path)
                }
            } else {
                join_path(&parsed.path, &percent_encode_path(stripped))
            }
        }
    };
    Ok(url_value(rebuild_with_path(&parsed, &new_path)))
}

/// Label-aware `appending(path:)` / `appending(component:)` — non-mutating.
fn url_appending_labeled(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let [arg] = args.as_slice() else {
        return Ok(None); // wrong arity — fall through
    };
    if !matches!(
        arg.label.as_deref(),
        Some("path") | Some("component") | None
    ) {
        return Ok(None);
    }
    let result = do_appending(&recv, arg)?;
    Ok(Some(Outcome {
        result,
        receiver: recv,
    }))
}

/// Label-aware `append(path:)` / `append(component:)` — mutating.
fn url_append_labeled(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let [arg] = args.as_slice() else {
        return Ok(None);
    };
    if !matches!(
        arg.label.as_deref(),
        Some("path") | Some("component") | None
    ) {
        return Ok(None);
    }
    let updated = do_appending(&recv, arg)?;
    Ok(Some(Outcome {
        result: SwiftValue::Void,
        receiver: updated,
    }))
}

// ---- appendPathExtension / deletePathExtension ---------------------------

/// Mutating `appendPathExtension(_:)` — mirrors the existing non-mutating
/// `appendingPathExtension` but writes back to the receiver.
fn url_append_path_extension(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let [SwiftValue::Str(ext)] = args.as_slice() else {
        return Err(type_error(
            "URL.appendPathExtension(_:) expects exactly one String argument",
        ));
    };
    if ext.is_empty() {
        // Empty extension is a no-op per Foundation.
        return Ok(Outcome {
            result: SwiftValue::Void,
            receiver: recv,
        });
    }
    let parsed = parse_url(&url_string(&recv)?);
    let had_slash = parsed.path.ends_with('/');
    let stem = parsed.path.trim_end_matches('/');
    let new_path = if had_slash {
        format!("{stem}.{ext}/")
    } else {
        format!("{stem}.{ext}")
    };
    let updated = url_value(rebuild_with_path(&parsed, &new_path));
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: updated,
    })
}

/// Mutating `deletePathExtension()` — mirrors `deletingPathExtension`.
fn url_delete_path_extension(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !args.is_empty() {
        return Err(type_error("URL.deletePathExtension() takes no arguments"));
    }
    let parsed = parse_url(&url_string(&recv)?);
    let path = &parsed.path;
    let new_path = match path.rsplit_once('/') {
        Some((head, last)) => match file_extension(last) {
            Some((stem, _)) => format!("{head}/{stem}"),
            None => path.clone(),
        },
        None => match file_extension(path) {
            Some((stem, _)) => stem.to_string(),
            None => path.clone(),
        },
    };
    let updated = url_value(rebuild_with_path(&parsed, &new_path));
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: updated,
    })
}

// ---- standardized / standardize() ----------------------------------------

/// Read-only `standardized` property.
fn url_standardized(recv: SwiftValue) -> StdResult {
    let parsed = parse_url(&url_string(&recv)?);
    let new_path = standardize_path(&parsed.path);
    Ok(url_value(rebuild_with_path(&parsed, &new_path)))
}

/// Mutating `standardize()`.
fn url_standardize(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !args.is_empty() {
        return Err(type_error("URL.standardize() takes no arguments"));
    }
    let parsed = parse_url(&url_string(&recv)?);
    let new_path = standardize_path(&parsed.path);
    let updated = url_value(rebuild_with_path(&parsed, &new_path));
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: updated,
    })
}

// ---- absoluteURL ---------------------------------------------------------

/// `absoluteURL` — since we only model absolute URLs, returns self.
fn url_absolute_url(recv: SwiftValue) -> StdResult {
    Ok(recv)
}

// ---- resolvingSymlinksInPath / resolveSymlinksInPath ---------------------

/// Lexical symlink resolution for `URL.resolvingSymlinksInPath()`.
///
/// For paths that do not exist on disk (the common case in a pure-Rust
/// interpreter with no filesystem access) Foundation documents that only
/// `.` and `..` are resolved — the same behaviour as `standardized`.
/// No `/private` prefix stripping is performed here because that mapping
/// is a real-filesystem operation (readlink on macOS expands `/tmp` to
/// `/private/tmp`); without a real symlink to resolve the path is left
/// verbatim after standardization.
fn resolve_symlinks(recv: &SwiftValue) -> Result<SwiftValue, StdError> {
    let s = url_string(recv)?;
    let parsed = parse_url(&s);
    let new_path = standardize_path(&parsed.path);
    Ok(url_value(rebuild_with_path(&parsed, &new_path)))
}

fn url_resolving_symlinks_in_path(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !args.is_empty() {
        return Err(type_error(
            "URL.resolvingSymlinksInPath() takes no arguments",
        ));
    }
    let result = resolve_symlinks(&recv)?;
    Ok(Outcome {
        result,
        receiver: recv,
    })
}

fn url_resolve_symlinks_in_path(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if !args.is_empty() {
        return Err(type_error("URL.resolveSymlinksInPath() takes no arguments"));
    }
    let updated = resolve_symlinks(&recv)?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: updated,
    })
}

// ===========================================================================
// URLQueryItem
// ===========================================================================

fn url_query_item_value(name: String, value: Option<String>) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "URLQueryItem".into(),
        fields: vec![
            ("name".into(), SwiftValue::Str(name)),
            (
                "value".into(),
                value.map(SwiftValue::Str).unwrap_or(SwiftValue::Nil),
            ),
        ],
    }))
}

fn url_query_item_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut name = None;
    let mut value: Option<Option<String>> = None;
    for arg in &args {
        match arg.label.as_deref() {
            Some("name") => match &arg.value {
                SwiftValue::Str(s) => name = Some(s.clone()),
                _ => return Err(type_error("URLQueryItem name must be a String")),
            },
            Some("value") => {
                value = Some(match &arg.value {
                    SwiftValue::Str(s) => Some(s.clone()),
                    SwiftValue::Nil => None,
                    _ => return Err(type_error("URLQueryItem value must be String?")),
                })
            }
            _ => {
                return Err(type_error(
                    "URLQueryItem(name:value:) expects labelled arguments",
                ))
            }
        }
    }
    let Some(name) = name else {
        return Err(type_error("URLQueryItem requires a name:"));
    };
    let Some(value) = value else {
        return Err(type_error("URLQueryItem requires a value: label"));
    };
    Ok(url_query_item_value(name, value))
}

// ===========================================================================
// URLComponents
// ===========================================================================

fn url_components_value(parsed: &Parsed) -> SwiftValue {
    // Store all text components in their **percent-decoded** form so that
    // the plain getters (`.path`, `.query`, …) return the user-visible decoded
    // string and the percent-encoded getters re-encode on demand.
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "URLComponents".into(),
        fields: vec![
            ("scheme".into(), opt_str(parsed.scheme.clone())),
            (
                "user".into(),
                opt_str(parsed.user.as_deref().map(percent_decode)),
            ),
            (
                "password".into(),
                opt_str(parsed.password.as_deref().map(percent_decode)),
            ),
            // Host is stored as-is (ASCII hosts need no decoding; IDNA is
            // out of scope).
            ("host".into(), opt_str(parsed.host.clone())),
            (
                "port".into(),
                parsed.port.map(SwiftValue::int).unwrap_or(SwiftValue::Nil),
            ),
            ("path".into(), SwiftValue::Str(percent_decode(&parsed.path))),
            // `query` is the canonical query store; `queryItems` is a read-only
            // property derived from it (avoids the two drifting out of sync).
            (
                "query".into(),
                opt_str(parsed.query.as_deref().map(percent_decode)),
            ),
            (
                "fragment".into(),
                opt_str(parsed.fragment.as_deref().map(percent_decode)),
            ),
        ],
    }))
}

fn url_components_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if args.is_empty() {
        return Ok(url_components_value(&Parsed::default()));
    }
    if args.len() != 1 {
        return Err(type_error("URLComponents expects zero or one argument"));
    }
    match args[0].label.as_deref() {
        Some("string") => {
            let SwiftValue::Str(raw) = &args[0].value else {
                return Err(type_error("URLComponents(string:) expects a String"));
            };
            Ok(url_components_value(&parse_url(raw)))
        }
        Some("url") => Ok(url_components_value(&parse_url(&url_string(
            &args[0].value,
        )?))),
        Some(other) => Err(type_error(format!(
            "unsupported URLComponents argument {other}:"
        ))),
        None => Err(type_error("URLComponents argument needs a label")),
    }
}

/// `queryItems` (read-only here): parse the canonical `query` field into
/// `[URLQueryItem]?`. Writing `queryItems` directly is not supported — set
/// `query` instead.
fn url_components_query_items(recv: SwiftValue) -> StdResult {
    Ok(query_items_from(comp_str(&recv, "query").as_deref()))
}

/// Build a `[URLQueryItem]?` from a **decoded** query string.
///
/// The stored query field is already decoded (see [`url_components_value`]),
/// so splitting on `&`/`=` and returning items as-is (no additional
/// `percent_decode` needed) is correct for the common case. A literal `&` or
/// `=` inside a name or value — which would have been `%26`/`%3D`-encoded in
/// the source URL — is a known limitation documented in notes.md.
fn query_items_from(query: Option<&str>) -> SwiftValue {
    let Some(query) = query else {
        return SwiftValue::Nil;
    };
    if query.is_empty() {
        return SwiftValue::Nil;
    }
    let items: Vec<SwiftValue> = query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => url_query_item_value(k.to_string(), Some(v.to_string())),
            None => url_query_item_value(pair.to_string(), None),
        })
        .collect();
    SwiftValue::Array(Rc::new(items))
}

/// Read a component field from a URLComponents struct value.
fn comp_field<'a>(value: &'a SwiftValue, name: &str) -> Option<&'a SwiftValue> {
    match value {
        SwiftValue::Struct(o) if o.type_name == "URLComponents" => o.get(name),
        _ => None,
    }
}

fn comp_str(value: &SwiftValue, name: &str) -> Option<String> {
    match comp_field(value, name) {
        Some(SwiftValue::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Assemble the URL string from the current component fields.
///
/// The stored fields hold **decoded** strings (see [`url_components_value`]),
/// so each text component must be re-encoded with the appropriate RFC 3986
/// character set before being placed into the URL.
fn components_to_string(value: &SwiftValue) -> String {
    let raw_user = comp_str(value, "user");
    let raw_password = comp_str(value, "password");
    let raw_path = comp_str(value, "path").unwrap_or_default();
    let raw_query = comp_str(value, "query");
    let raw_fragment = comp_str(value, "fragment");

    // Host in the URL string:
    // - If `_encodedHost` is set (via the `encodedHost` setter), use it verbatim
    //   so Foundation's IDNA / verbatim-encoding semantics are preserved.
    // - Otherwise encode the decoded `host` field with the host allowed charset.
    let decoded_host = comp_str(value, "host");
    // Use the verbatim `_encodedHost` string only when it is non-empty; when
    // the field is absent or Nil (override cleared) fall back to re-encoding
    // the decoded host.
    let url_host = match comp_field(value, "_encodedHost") {
        Some(SwiftValue::Str(s)) if !s.is_empty() => Some(s.clone()),
        _ => decoded_host.as_deref().map(|h| h.to_owned()), // ASCII host: no re-encoding
    };

    let parsed = Parsed {
        scheme: comp_str(value, "scheme"),
        // Emit an authority whenever a host or user is set.
        authority: decoded_host.is_some() || raw_user.is_some(),
        user: raw_user.as_deref().map(percent_encode_userinfo),
        password: raw_password.as_deref().map(percent_encode_userinfo),
        host: url_host,
        port: match comp_field(value, "port") {
            Some(SwiftValue::Int(i)) => Some(i.raw),
            _ => None,
        },
        path: percent_encode_path(&raw_path),
        query: raw_query.as_deref().map(percent_encode_query),
        fragment: raw_fragment.as_deref().map(percent_encode_query),
    };
    rebuild_with_path(&parsed, &parsed.path)
}

fn url_components_string(recv: SwiftValue) -> StdResult {
    Ok(SwiftValue::Str(components_to_string(&recv)))
}

fn url_components_url(recv: SwiftValue) -> StdResult {
    let string = components_to_string(&recv);
    if string.is_empty() {
        Ok(SwiftValue::Nil)
    } else {
        Ok(url_value(string))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_authority() {
        let p = parse_url("https://u:pw@h.com:80/a/b?x=1#f");
        assert_eq!(p.scheme.as_deref(), Some("https"));
        assert_eq!(p.user.as_deref(), Some("u"));
        assert_eq!(p.password.as_deref(), Some("pw"));
        assert_eq!(p.host.as_deref(), Some("h.com"));
        assert_eq!(p.port, Some(80));
        assert_eq!(p.path, "/a/b");
        assert_eq!(p.query.as_deref(), Some("x=1"));
        assert_eq!(p.fragment.as_deref(), Some("f"));
    }

    #[test]
    fn parses_scheme_only_and_no_authority() {
        let p = parse_url("mailto:a@b.com");
        assert_eq!(p.scheme.as_deref(), Some("mailto"));
        assert_eq!(p.host, None);
        assert_eq!(p.path, "a@b.com");
    }

    #[test]
    fn parses_host_without_port_or_userinfo() {
        let p = parse_url("https://host.com/x");
        assert_eq!(p.host.as_deref(), Some("host.com"));
        assert_eq!(p.port, None);
        assert_eq!(p.user, None);
        assert_eq!(p.path, "/x");
    }

    #[test]
    fn last_component_handles_trailing_slash_and_root() {
        assert_eq!(last_component("/a/b/"), "b");
        assert_eq!(last_component("/a/b"), "b");
        assert_eq!(last_component("/"), "/");
        assert_eq!(last_component(""), "");
    }

    #[test]
    fn round_trips_rebuild() {
        let original = "https://u:pw@h.com:80/a/b?x=1#f";
        let p = parse_url(original);
        assert_eq!(rebuild_with_path(&p, &p.path), original);
    }

    #[test]
    fn preserves_empty_authority() {
        let p = parse_url("file:///tmp/a");
        assert!(p.authority);
        assert_eq!(p.host, None);
        assert_eq!(p.path, "/tmp/a");
        assert_eq!(rebuild_with_path(&p, &p.path), "file:///tmp/a");
    }

    #[test]
    fn parses_ipv6_host_and_port() {
        let p = parse_url("http://[::1]:9000/x");
        assert_eq!(p.host.as_deref(), Some("[::1]"));
        assert_eq!(p.port, Some(9000));
    }

    #[test]
    fn rejects_non_numeric_port() {
        assert_eq!(parse_url("http://h:-1/").port, None);
        assert_eq!(parse_url("http://h:abc/").port, None);
    }

    #[test]
    fn file_extension_handles_dotfiles() {
        assert_eq!(file_extension("a.txt"), Some(("a", "txt")));
        assert_eq!(file_extension("a.b.txt"), Some(("a.b", "txt")));
        assert_eq!(file_extension(".bashrc"), None);
        assert_eq!(file_extension("a."), None);
        assert_eq!(file_extension("plain"), None);
    }

    #[test]
    fn standardize_path_resolves_dot_and_dotdot() {
        assert_eq!(standardize_path("/a/b/../c"), "/a/c");
        assert_eq!(standardize_path("/a/./b"), "/a/b");
        // double slashes are PRESERVED when no '..' is involved
        assert_eq!(standardize_path("/a//b"), "/a//b");
        // '..' clamped at root for absolute paths
        assert_eq!(standardize_path("/../../a"), "/a");
        // '..' at root must yield '/', never an empty string
        assert_eq!(standardize_path("/.."), "/");
        assert_eq!(standardize_path("/../.."), "/");
        // '.' at root is also '/' not ''
        assert_eq!(standardize_path("/."), "/");
        // relative path keeps unresolvable '..'
        assert_eq!(standardize_path("../a"), "../a");
        // trailing slash preserved
        assert_eq!(standardize_path("/a/b/../"), "/a/");
        // '..' after empty segment (double-slash): pops the empty, resolves correctly
        assert_eq!(standardize_path("/a//../c"), "/a/c");
        assert_eq!(standardize_path("/a//b/../c"), "/a//c");
        assert_eq!(standardize_path("/a//b/../../c"), "/a/c");
    }

    #[test]
    fn percent_decode_decodes_escapes() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("%41%42"), "AB");
        // An invalid escape is left intact.
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("a%zzb"), "a%zzb");
    }
}
