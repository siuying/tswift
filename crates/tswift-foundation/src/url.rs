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
    Arg, BuiltinReceiver, MethodEntry, Outcome, StdContext, StdError, StdResult, StructObj,
    SwiftValue,
};

use crate::type_error;

/// Register the URL family on `interp`.
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
    ] {
        interp.register_intrinsic(
            BuiltinReceiver::URL,
            name,
            MethodEntry { mutating, func: f },
        );
    }

    // ---- URLQueryItem ----
    interp.register_free_fn("URLQueryItem", url_query_item_init);

    // ---- URLComponents ----
    interp.register_free_fn("URLComponents", url_components_init);
    interp.register_property(BuiltinReceiver::URLComponents, "url", url_components_url);
    interp.register_property(
        BuiltinReceiver::URLComponents,
        "string",
        url_components_string,
    );
}

// ===========================================================================
// URL value model
// ===========================================================================

#[derive(Default, Clone)]
struct Parsed {
    scheme: Option<String>,
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
        if let Some(colon) = host_port.rfind(':') {
            let host = &host_port[..colon];
            if !host.is_empty() {
                p.host = Some(host.to_string());
            }
            if let Ok(port) = host_port[colon + 1..].parse::<i128>() {
                p.port = Some(port);
            }
        } else if !host_port.is_empty() {
            p.host = Some(host_port.to_string());
        }
    }
    p.path = rest.to_string();
    p
}

fn url_value(string: String, is_file: bool) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "URL".into(),
        fields: vec![
            ("_string".into(), SwiftValue::Str(string)),
            ("_isFile".into(), SwiftValue::Bool(is_file)),
        ],
    }))
}

fn url_string(value: &SwiftValue) -> Result<String, StdError> {
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

fn url_is_file_flag(value: &SwiftValue) -> bool {
    matches!(value, SwiftValue::Struct(o) if matches!(o.get("_isFile"), Some(SwiftValue::Bool(true))))
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
        // `URL(string:)` is failable: an empty string yields `nil`.
        Some("string") => Ok(if raw.is_empty() {
            SwiftValue::Nil
        } else {
            url_value(raw.clone(), false)
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
            Ok(url_value(string, true))
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
    Ok(SwiftValue::Str(parse_url(&url_string(&recv)?).path))
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
    let ext = last
        .rsplit_once('.')
        .map(|(_, e)| e.to_string())
        .unwrap_or_default();
    Ok(SwiftValue::Str(ext))
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
    if parsed.host.is_some() || parsed.user.is_some() {
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
    let new_path = join_path(&parsed.path, comp);
    Ok(url_value(
        rebuild_with_path(&parsed, &new_path),
        url_is_file_flag(recv),
    ))
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
    Ok(url_value(
        rebuild_with_path(&parsed, &new_path),
        url_is_file_flag(recv),
    ))
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
    let new_path = format!("{}.{}", parsed.path.trim_end_matches('/'), ext);
    let result = url_value(
        rebuild_with_path(&parsed, &new_path),
        url_is_file_flag(&recv),
    );
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
        Some((head, last)) => match last.rsplit_once('.') {
            Some((stem, _)) => format!("{head}/{stem}"),
            None => path.clone(),
        },
        None => match path.rsplit_once('.') {
            Some((stem, _)) => stem.to_string(),
            None => path.clone(),
        },
    };
    let result = url_value(
        rebuild_with_path(&parsed, &new_path),
        url_is_file_flag(&recv),
    );
    Ok(Outcome {
        result,
        receiver: recv,
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
        return Err(type_error("URLQueryItem requires a name"));
    };
    Ok(url_query_item_value(name, value.flatten()))
}

// ===========================================================================
// URLComponents
// ===========================================================================

fn url_components_value(parsed: &Parsed, query_items: SwiftValue) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "URLComponents".into(),
        fields: vec![
            ("scheme".into(), opt_str(parsed.scheme.clone())),
            ("user".into(), opt_str(parsed.user.clone())),
            ("password".into(), opt_str(parsed.password.clone())),
            ("host".into(), opt_str(parsed.host.clone())),
            (
                "port".into(),
                parsed.port.map(SwiftValue::int).unwrap_or(SwiftValue::Nil),
            ),
            ("path".into(), SwiftValue::Str(parsed.path.clone())),
            ("query".into(), opt_str(parsed.query.clone())),
            ("fragment".into(), opt_str(parsed.fragment.clone())),
            ("queryItems".into(), query_items),
        ],
    }))
}

fn url_components_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if args.is_empty() {
        return Ok(url_components_value(&Parsed::default(), SwiftValue::Nil));
    }
    if args.len() != 1 {
        return Err(type_error("URLComponents expects zero or one argument"));
    }
    match args[0].label.as_deref() {
        Some("string") => {
            let SwiftValue::Str(raw) = &args[0].value else {
                return Err(type_error("URLComponents(string:) expects a String"));
            };
            let parsed = parse_url(raw);
            let items = query_items_from(parsed.query.as_deref());
            Ok(url_components_value(&parsed, items))
        }
        Some("url") => {
            let parsed = parse_url(&url_string(&args[0].value)?);
            let items = query_items_from(parsed.query.as_deref());
            Ok(url_components_value(&parsed, items))
        }
        Some(other) => Err(type_error(format!(
            "unsupported URLComponents argument {other}:"
        ))),
        None => Err(type_error("URLComponents argument needs a label")),
    }
}

/// Build a `[URLQueryItem]?` from a raw query string.
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

/// Recompute the query string from the current `queryItems`, if any.
fn query_from_items(value: &SwiftValue) -> Option<String> {
    let SwiftValue::Array(items) = comp_field(value, "queryItems")? else {
        return None;
    };
    if items.is_empty() {
        return Some(String::new());
    }
    let parts: Vec<String> = items
        .iter()
        .filter_map(|item| {
            let SwiftValue::Struct(o) = item else {
                return None;
            };
            let name = match o.get("name") {
                Some(SwiftValue::Str(s)) => s.clone(),
                _ => return None,
            };
            Some(match o.get("value") {
                Some(SwiftValue::Str(v)) => format!("{name}={v}"),
                _ => name,
            })
        })
        .collect();
    Some(parts.join("&"))
}

/// Assemble the URL string from the current component fields.
fn components_to_string(value: &SwiftValue) -> String {
    let parsed = Parsed {
        scheme: comp_str(value, "scheme"),
        user: comp_str(value, "user"),
        password: comp_str(value, "password"),
        host: comp_str(value, "host"),
        port: match comp_field(value, "port") {
            Some(SwiftValue::Int(i)) => Some(i.raw),
            _ => None,
        },
        path: comp_str(value, "path").unwrap_or_default(),
        query: query_from_items(value).or_else(|| comp_str(value, "query")),
        fragment: comp_str(value, "fragment"),
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
        Ok(url_value(string, false))
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
}
