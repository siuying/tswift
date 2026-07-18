//! Foundation networking value types: `URLRequest`, `URLResponse`,
//! `HTTPURLResponse`, and `URLError`.
//!
//! All are modelled as `SwiftValue::Struct` with public-named stored fields so
//! plain member reads/writes flow through the generic struct member path
//! (mirroring `URLComponents`). Header dictionaries are `[String: String]`
//! with **case-insensitive** field-name lookup, matching Foundation.
//!
//! `URLSession` itself lives behind the transport seam (see the plan in
//! `docs/plan/framework-support.md`); this module is the pure value layer it
//! builds on.

use std::rc::Rc;

use tswift_core::{
    Arg, BuiltinReceiver, EnumObj, LabeledMethodEntry, MethodEntry, Outcome, StdContext, StdError,
    StdResult, StructObj, SwiftValue,
};

use crate::type_error;
use crate::url::url_string;

/// `URLError.Code` cases in canonical order, with their `NSURLError*` raw
/// values (`errorCode`).
pub(crate) const URL_ERROR_CODES: &[(&str, i128)] = &[
    ("unknown", -1),
    ("cancelled", -999),
    ("badURL", -1000),
    ("timedOut", -1001),
    ("unsupportedURL", -1002),
    ("cannotFindHost", -1003),
    ("cannotConnectToHost", -1004),
    ("networkConnectionLost", -1005),
    ("dnsLookupFailed", -1006),
    ("httpTooManyRedirects", -1007),
    ("resourceUnavailable", -1008),
    ("notConnectedToInternet", -1009),
    ("redirectToNonExistentLocation", -1010),
    ("badServerResponse", -1011),
    ("userCancelledAuthentication", -1012),
    ("userAuthenticationRequired", -1013),
    ("zeroByteResource", -1014),
    ("cannotDecodeRawData", -1015),
    ("cannotDecodeContentData", -1016),
    ("cannotParseResponse", -1017),
    ("internationalRoamingOff", -1018),
    ("callIsActive", -1019),
    ("dataNotAllowed", -1020),
    ("requestBodyStreamExhausted", -1021),
    ("appTransportSecurityRequiresSecureConnection", -1022),
    ("fileDoesNotExist", -1100),
    ("fileIsDirectory", -1101),
    ("noPermissionsToReadFile", -1102),
    ("dataLengthExceedsMaximum", -1103),
    ("secureConnectionFailed", -1200),
    ("serverCertificateHasBadDate", -1201),
    ("serverCertificateUntrusted", -1202),
    ("serverCertificateHasUnknownRoot", -1203),
    ("serverCertificateNotYetValid", -1204),
    ("clientCertificateRejected", -1205),
    ("clientCertificateRequired", -1206),
    ("cannotLoadFromNetwork", -2000),
    ("cannotCreateFile", -3000),
    ("cannotOpenFile", -3001),
    ("cannotCloseFile", -3002),
    ("cannotWriteToFile", -3003),
    ("cannotRemoveFile", -3004),
    ("cannotMoveFile", -3005),
    ("downloadDecodingFailedMidStream", -3006),
    ("downloadDecodingFailedToComplete", -3007),
    ("backgroundSessionRequiresSharedContainer", -995),
    ("backgroundSessionInUseByAnotherProcess", -996),
    ("backgroundSessionWasDisconnected", -997),
];

/// Register the networking value types on `interp`.
pub(crate) fn install(interp: &mut tswift_core::Interpreter<'_>) {
    // ---- URLRequest ----
    interp.register_free_fn("URLRequest", url_request_init);
    for (name, f) in [
        ("url", url_request_url as fn(SwiftValue) -> StdResult),
        ("httpMethod", url_request_http_method),
        ("httpBody", url_request_http_body),
        ("timeoutInterval", url_request_timeout_interval),
        ("allHTTPHeaderFields", url_request_all_header_fields),
        ("description", url_request_description),
        ("debugDescription", url_request_description),
        ("hashValue", url_request_hash_value),
        ("cachePolicy", url_request_cache_policy),
        ("networkServiceType", url_request_network_service_type),
        ("attribution", url_request_attribution),
        ("allowsCellularAccess", url_request_allows_cellular_access),
        (
            "allowsConstrainedNetworkAccess",
            url_request_allows_constrained_network_access,
        ),
        (
            "allowsExpensiveNetworkAccess",
            url_request_allows_expensive_network_access,
        ),
        (
            "httpShouldHandleCookies",
            url_request_http_should_handle_cookies,
        ),
        (
            "httpShouldUsePipelining",
            url_request_http_should_use_pipelining,
        ),
        ("assumesHTTP3Capable", url_request_assumes_http3_capable),
        (
            "requiresDNSSECValidation",
            url_request_requires_dnssec_validation,
        ),
        ("allowsPersistentDNS", url_request_allows_persistent_dns),
        (
            "allowsUltraConstrainedNetworkAccess",
            url_request_allows_ultra_constrained_network_access,
        ),
        ("mainDocumentURL", url_request_main_document_url),
        (
            "cookiePartitionIdentifier",
            url_request_cookie_partition_identifier,
        ),
    ] {
        interp.register_property(BuiltinReceiver::URLRequest, name, f);
    }
    // Plain stored-property setters: reads flow through the generic struct
    // member path (`obj.get(name)`), but a `Type.member` shorthand like
    // `.reloadIgnoringLocalCacheData` on the right-hand side of an assignment
    // needs the field's declared type to resolve; the generic struct-field
    // setter (no coercion) already handles the write itself.
    interp.register_builtin_enum_with_raw(
        "URLRequest.CachePolicy",
        &[
            ("useProtocolCachePolicy", 0),
            ("reloadIgnoringLocalCacheData", 1),
            ("returnCacheDataElseLoad", 2),
            ("returnCacheDataDontLoad", 3),
            ("reloadIgnoringLocalAndRemoteCacheData", 4),
            ("reloadRevalidatingCacheData", 5),
        ],
    );
    interp.register_builtin_enum_with_raw(
        "URLRequest.NetworkServiceType",
        &[
            ("default", 0),
            ("voip", 1),
            ("video", 2),
            ("background", 3),
            ("voice", 4),
            ("responsiveData", 6),
            ("avStreaming", 8),
            ("responsiveAV", 9),
            ("callSignaling", 11),
        ],
    );
    interp
        .register_builtin_enum_with_raw("URLRequest.Attribution", &[("developer", 0), ("user", 1)]);
    interp.register_intrinsic(
        BuiltinReceiver::URLRequest,
        "==",
        MethodEntry {
            mutating: false,
            func: url_request_equal,
        },
    );
    // `timeoutInterval` is a `TimeInterval` (Double); coerce Int assignments
    // so `req.timeoutInterval = 30` reads back as `30.0` like Foundation.
    interp.register_property_setter(
        BuiltinReceiver::URLRequest,
        "timeoutInterval",
        url_request_set_timeout_interval,
    );
    interp.register_labeled_intrinsic(
        BuiltinReceiver::URLRequest,
        "setValue",
        LabeledMethodEntry {
            mutating: true,
            func: url_request_set_value,
        },
    );
    interp.register_labeled_intrinsic(
        BuiltinReceiver::URLRequest,
        "addValue",
        LabeledMethodEntry {
            mutating: true,
            func: url_request_add_value,
        },
    );
    interp.register_labeled_intrinsic(
        BuiltinReceiver::URLRequest,
        "value",
        LabeledMethodEntry {
            mutating: false,
            func: url_request_value_for_field,
        },
    );

    // ---- URLResponse ----
    interp.register_free_fn("URLResponse", url_response_init);
    for (name, f) in [
        ("url", response_url as fn(SwiftValue) -> StdResult),
        ("mimeType", response_mime_type),
        ("expectedContentLength", response_expected_content_length),
        ("textEncodingName", response_text_encoding_name),
        ("suggestedFilename", response_suggested_filename),
        ("description", response_description),
        ("debugDescription", response_description),
    ] {
        interp.register_property(BuiltinReceiver::URLResponse, name, f);
        interp.register_property(BuiltinReceiver::HTTPURLResponse, name, f);
    }

    // ---- HTTPURLResponse ----
    interp.register_free_fn("HTTPURLResponse", http_url_response_init);
    for (name, f) in [
        (
            "statusCode",
            http_response_status_code as fn(SwiftValue) -> StdResult,
        ),
        ("allHeaderFields", http_response_all_header_fields),
    ] {
        interp.register_property(BuiltinReceiver::HTTPURLResponse, name, f);
    }
    interp.register_labeled_intrinsic(
        BuiltinReceiver::HTTPURLResponse,
        "value",
        LabeledMethodEntry {
            mutating: false,
            func: http_response_value_for_field,
        },
    );
    interp.register_static(
        BuiltinReceiver::HTTPURLResponse,
        "localizedString",
        http_response_localized_string,
    );

    // ---- URLError ----
    interp.register_builtin_enum_with_raw("URLError.Code", URL_ERROR_CODES);
    interp.register_free_fn("URLError", url_error_init);
    for (name, f) in [
        ("code", url_error_code as fn(SwiftValue) -> StdResult),
        ("errorCode", url_error_error_code),
        ("localizedDescription", url_error_localized_description),
        ("description", url_error_localized_description),
        ("failingURL", url_error_failing_url),
        ("hashValue", url_error_hash_value),
        ("failureURLString", url_error_failure_url_string),
        ("failureURLPeerTrust", url_error_failure_url_peer_trust),
        (
            "networkUnavailableReason",
            url_error_network_unavailable_reason,
        ),
        (
            "backgroundTaskCancelledReason",
            url_error_background_task_cancelled_reason,
        ),
        (
            "downloadTaskResumeData",
            url_error_download_task_resume_data,
        ),
        ("uploadTaskResumeData", url_error_upload_task_resume_data),
    ] {
        interp.register_property(BuiltinReceiver::URLError, name, f);
    }
    // `errorDomain` is a static constant (`NSErrorDomain`), always the same
    // string; not a per-instance property.
    interp.register_static_value(
        "URLError",
        "errorDomain",
        SwiftValue::Str("NSURLErrorDomain".to_string()),
    );
    // `URLError.Code.badURL` and contextual `.badURL` resolve via the builtin
    // enum registration above; also surface each case as `URLError.badURL`
    // (Foundation exposes the codes on `URLError` too).
    for (case, _) in URL_ERROR_CODES {
        interp.register_static_value("URLError", case, url_error_code_value(case));
    }
}

/// Statics-table keys for the coverage registry: `URLError.Code` cases plus
/// the session/configuration static values, which `Interpreter::
/// registered_keys` does not report (they are not builtin members).
pub(crate) fn extra_registered_keys() -> Vec<String> {
    let mut keys: Vec<String> = URL_ERROR_CODES
        .iter()
        .map(|(case, _)| format!("URLError.{case}"))
        .collect();
    keys.push("URLError.errorDomain".to_string());
    keys.push("URLSession.shared".to_string());
    keys.push("URLSessionConfiguration.default".to_string());
    keys.push("URLSessionConfiguration.ephemeral".to_string());
    keys
}

// ===========================================================================
// URLRequest
// ===========================================================================

/// The default `URLRequest.CachePolicy` (`.useProtocolCachePolicy`).
fn default_cache_policy() -> SwiftValue {
    url_request_enum_case("URLRequest.CachePolicy", "useProtocolCachePolicy")
}

/// The default `URLRequest.NetworkServiceType` (`.default`).
fn default_network_service_type() -> SwiftValue {
    url_request_enum_case("URLRequest.NetworkServiceType", "default")
}

/// The default `URLRequest.Attribution` (`.developer`).
fn default_attribution() -> SwiftValue {
    url_request_enum_case("URLRequest.Attribution", "developer")
}

fn url_request_enum_case(type_name: &str, case: &str) -> SwiftValue {
    SwiftValue::Enum(Rc::new(EnumObj {
        type_name: type_name.into(),
        case: case.into(),
        payload: Vec::new(),
    }))
}

/// Build the canonical URLRequest struct value, with every Darwin-default
/// stored field (cache policy, cellular/network-access flags, …) filled in.
pub(crate) fn url_request_value(
    url: SwiftValue,
    timeout: f64,
    headers: SwiftValue,
    method: SwiftValue,
    body: SwiftValue,
) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "URLRequest".into(),
        fields: vec![
            ("url".into(), url),
            ("httpMethod".into(), method),
            ("httpBody".into(), body),
            ("timeoutInterval".into(), SwiftValue::Double(timeout)),
            ("allHTTPHeaderFields".into(), headers),
            ("cachePolicy".into(), default_cache_policy()),
            ("networkServiceType".into(), default_network_service_type()),
            ("attribution".into(), default_attribution()),
            ("allowsCellularAccess".into(), SwiftValue::Bool(true)),
            (
                "allowsConstrainedNetworkAccess".into(),
                SwiftValue::Bool(true),
            ),
            (
                "allowsExpensiveNetworkAccess".into(),
                SwiftValue::Bool(true),
            ),
            ("httpShouldHandleCookies".into(), SwiftValue::Bool(true)),
            ("httpShouldUsePipelining".into(), SwiftValue::Bool(false)),
            ("assumesHTTP3Capable".into(), SwiftValue::Bool(false)),
            ("requiresDNSSECValidation".into(), SwiftValue::Bool(false)),
            // Verified against real Apple swiftc 6.3.2: the actual Darwin
            // default is `false`, not `true` (the task brief's assumed
            // default was wrong — flagged in notes.md).
            ("allowsPersistentDNS".into(), SwiftValue::Bool(false)),
            // Verified against real Apple swiftc 6.3.2 (macOS 26.1 SDK): the
            // actual Darwin default is `false`, not `true` (the task brief's
            // assumed default was wrong — flagged in notes.md).
            (
                "allowsUltraConstrainedNetworkAccess".into(),
                SwiftValue::Bool(false),
            ),
            ("mainDocumentURL".into(), SwiftValue::Nil),
            ("cookiePartitionIdentifier".into(), SwiftValue::Nil),
        ],
    }))
}

fn url_request_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut url = None;
    let mut timeout = 60.0;
    let mut cache_policy = None;
    for arg in &args {
        match arg.label.as_deref() {
            Some("url") => {
                url_string(&arg.value)?; // validate it is a URL
                url = Some(arg.value.clone());
            }
            Some("timeoutInterval") => {
                timeout = match &arg.value {
                    SwiftValue::Double(d) => *d,
                    SwiftValue::Int(i) => i.raw as f64,
                    _ => {
                        return Err(type_error(
                            "URLRequest timeoutInterval must be a TimeInterval",
                        ))
                    }
                }
            }
            Some("cachePolicy") => match &arg.value {
                SwiftValue::Enum(e) if e.type_name == "URLRequest.CachePolicy" => {
                    cache_policy = Some(arg.value.clone());
                }
                _ => return Err(type_error("cachePolicy must be a URLRequest.CachePolicy")),
            },
            Some(other) => {
                return Err(type_error(format!(
                    "unsupported URLRequest argument {other}:"
                )))
            }
            None => return Err(type_error("URLRequest arguments need labels")),
        }
    }
    let Some(url) = url else {
        return Err(type_error("URLRequest requires a url:"));
    };
    let mut req = url_request_value(
        url,
        timeout,
        SwiftValue::Nil,
        SwiftValue::Str("GET".into()),
        SwiftValue::Nil,
    );
    if let Some(policy) = cache_policy {
        let SwiftValue::Struct(o) = &req else {
            unreachable!("url_request_value always returns a Struct");
        };
        let mut obj = (**o).clone();
        obj.set("cachePolicy", policy);
        req = SwiftValue::Struct(Rc::new(obj));
    }
    Ok(req)
}

fn request_field(recv: &SwiftValue, name: &str) -> StdResult {
    match recv {
        SwiftValue::Struct(o) if o.type_name == "URLRequest" => {
            Ok(o.get(name).cloned().unwrap_or(SwiftValue::Nil))
        }
        _ => Err(type_error(format!("{name} expects URLRequest"))),
    }
}

fn url_request_url(recv: SwiftValue) -> StdResult {
    request_field(&recv, "url")
}

fn url_request_http_method(recv: SwiftValue) -> StdResult {
    request_field(&recv, "httpMethod")
}

fn url_request_http_body(recv: SwiftValue) -> StdResult {
    request_field(&recv, "httpBody")
}

fn url_request_timeout_interval(recv: SwiftValue) -> StdResult {
    request_field(&recv, "timeoutInterval")
}

fn url_request_all_header_fields(recv: SwiftValue) -> StdResult {
    request_field(&recv, "allHTTPHeaderFields")
}

fn url_request_set_timeout_interval(
    recv: SwiftValue,
    new_value: SwiftValue,
) -> Result<SwiftValue, StdError> {
    let seconds = match &new_value {
        SwiftValue::Double(d) => *d,
        SwiftValue::Int(i) => i.raw as f64,
        _ => return Err(type_error("timeoutInterval must be a TimeInterval")),
    };
    let SwiftValue::Struct(o) = &recv else {
        return Err(type_error("timeoutInterval expects URLRequest"));
    };
    let mut obj = (**o).clone();
    obj.set("timeoutInterval", SwiftValue::Double(seconds));
    Ok(SwiftValue::Struct(Rc::new(obj)))
}

fn url_request_description(recv: SwiftValue) -> StdResult {
    let url = request_field(&recv, "url")?;
    Ok(SwiftValue::Str(url_string(&url).unwrap_or_default()))
}

fn url_request_hash_value(recv: SwiftValue) -> StdResult {
    let url = request_field(&recv, "url")?;
    let method = request_field(&recv, "httpMethod")?;
    let mut bytes = url_string(&url).unwrap_or_default().into_bytes();
    bytes.push(0);
    if let SwiftValue::Str(m) = method {
        bytes.extend_from_slice(m.as_bytes());
    }
    Ok(SwiftValue::int(crate::fnv1a_hash(&bytes)))
}

fn url_request_cache_policy(recv: SwiftValue) -> StdResult {
    request_field(&recv, "cachePolicy")
}

fn url_request_network_service_type(recv: SwiftValue) -> StdResult {
    request_field(&recv, "networkServiceType")
}

fn url_request_attribution(recv: SwiftValue) -> StdResult {
    request_field(&recv, "attribution")
}

fn url_request_allows_cellular_access(recv: SwiftValue) -> StdResult {
    request_field(&recv, "allowsCellularAccess")
}

fn url_request_allows_constrained_network_access(recv: SwiftValue) -> StdResult {
    request_field(&recv, "allowsConstrainedNetworkAccess")
}

fn url_request_allows_expensive_network_access(recv: SwiftValue) -> StdResult {
    request_field(&recv, "allowsExpensiveNetworkAccess")
}

fn url_request_http_should_handle_cookies(recv: SwiftValue) -> StdResult {
    request_field(&recv, "httpShouldHandleCookies")
}

fn url_request_http_should_use_pipelining(recv: SwiftValue) -> StdResult {
    request_field(&recv, "httpShouldUsePipelining")
}

fn url_request_assumes_http3_capable(recv: SwiftValue) -> StdResult {
    request_field(&recv, "assumesHTTP3Capable")
}

fn url_request_requires_dnssec_validation(recv: SwiftValue) -> StdResult {
    request_field(&recv, "requiresDNSSECValidation")
}

fn url_request_allows_persistent_dns(recv: SwiftValue) -> StdResult {
    request_field(&recv, "allowsPersistentDNS")
}

fn url_request_allows_ultra_constrained_network_access(recv: SwiftValue) -> StdResult {
    request_field(&recv, "allowsUltraConstrainedNetworkAccess")
}

fn url_request_main_document_url(recv: SwiftValue) -> StdResult {
    request_field(&recv, "mainDocumentURL")
}

fn url_request_cookie_partition_identifier(recv: SwiftValue) -> StdResult {
    request_field(&recv, "cookiePartitionIdentifier")
}

/// Every stored field compared by `URLRequest.==` (`Equatable`), in the same
/// order `url_request_value` builds them.
const URL_REQUEST_EQUATABLE_FIELDS: &[&str] = &[
    "url",
    "httpMethod",
    "httpBody",
    "timeoutInterval",
    "allHTTPHeaderFields",
    "cachePolicy",
    "networkServiceType",
    "attribution",
    "allowsCellularAccess",
    "allowsConstrainedNetworkAccess",
    "allowsExpensiveNetworkAccess",
    "httpShouldHandleCookies",
    "httpShouldUsePipelining",
    "assumesHTTP3Capable",
    "requiresDNSSECValidation",
    "allowsPersistentDNS",
    "allowsUltraConstrainedNetworkAccess",
    "mainDocumentURL",
    "cookiePartitionIdentifier",
];

fn url_request_equal(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    if args.len() != 1 {
        return Err(type_error("URLRequest.== expects one URLRequest"));
    }
    let same = URL_REQUEST_EQUATABLE_FIELDS.iter().all(|field| {
        let lhs = request_field(&recv, field).unwrap_or(SwiftValue::Nil);
        let rhs = request_field(&args[0], field).unwrap_or(SwiftValue::Nil);
        lhs == rhs
    });
    Ok(Outcome {
        result: SwiftValue::Bool(same),
        receiver: recv,
    })
}

/// The stored header pairs of a request/response headers value (`Nil` → empty).
fn header_pairs(headers: &SwiftValue) -> Result<Vec<(String, String)>, StdError> {
    match headers {
        SwiftValue::Nil => Ok(Vec::new()),
        SwiftValue::Dict(pairs) => pairs
            .iter()
            .map(|(k, v)| match (k, v) {
                (SwiftValue::Str(k), SwiftValue::Str(v)) => Ok((k.clone(), v.clone())),
                _ => Err(type_error("HTTP header fields must be [String: String]")),
            })
            .collect(),
        _ => Err(type_error("HTTP header fields must be [String: String]")),
    }
}

fn headers_value(pairs: Vec<(String, String)>) -> SwiftValue {
    SwiftValue::Dict(Rc::new(
        pairs
            .into_iter()
            .map(|(k, v)| (SwiftValue::Str(k), SwiftValue::Str(v)))
            .collect(),
    ))
}

/// Case-insensitive header lookup, per RFC 7230 / Foundation semantics.
fn header_lookup(pairs: &[(String, String)], field: &str) -> Option<String> {
    pairs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(field))
        .map(|(_, v)| v.clone())
}

/// Extract the `(_ value, forHTTPHeaderField:)` argument pair.
fn value_and_field(args: &[Arg], method: &str) -> Result<(SwiftValue, String), StdError> {
    if args.len() != 2 || args[0].label.is_some() {
        return Err(type_error(format!(
            "{method} expects (_ value:, forHTTPHeaderField:)"
        )));
    }
    if args[1].label.as_deref() != Some("forHTTPHeaderField") {
        return Err(type_error(format!(
            "{method} expects forHTTPHeaderField: label"
        )));
    }
    let SwiftValue::Str(field) = &args[1].value else {
        return Err(type_error(format!("{method} field must be a String")));
    };
    Ok((args[0].value.clone(), field.clone()))
}

fn url_request_set_value(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let (value, field) = value_and_field(&args, "setValue")?;
    let mut pairs = header_pairs(&request_field(&recv, "allHTTPHeaderFields")?)?;
    match value {
        SwiftValue::Str(v) => {
            if let Some(slot) = pairs
                .iter_mut()
                .find(|(k, _)| k.eq_ignore_ascii_case(&field))
            {
                slot.1 = v;
            } else {
                pairs.push((field, v));
            }
        }
        // `setValue(nil, ...)` removes the field.
        SwiftValue::Nil => pairs.retain(|(k, _)| !k.eq_ignore_ascii_case(&field)),
        _ => return Err(type_error("setValue value must be String?")),
    }
    Ok(Some(Outcome {
        result: SwiftValue::Void,
        receiver: with_request_headers(&recv, pairs)?,
    }))
}

fn url_request_add_value(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    let (value, field) = value_and_field(&args, "addValue")?;
    let SwiftValue::Str(v) = value else {
        return Err(type_error("addValue value must be a String"));
    };
    let mut pairs = header_pairs(&request_field(&recv, "allHTTPHeaderFields")?)?;
    // Foundation merges repeated fields into one comma-joined value.
    if let Some(slot) = pairs
        .iter_mut()
        .find(|(k, _)| k.eq_ignore_ascii_case(&field))
    {
        slot.1 = format!("{},{v}", slot.1);
    } else {
        pairs.push((field, v));
    }
    Ok(Some(Outcome {
        result: SwiftValue::Void,
        receiver: with_request_headers(&recv, pairs)?,
    }))
}

fn url_request_value_for_field(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    if args.len() != 1 || args[0].label.as_deref() != Some("forHTTPHeaderField") {
        return Ok(None); // not our overload; let other `value` methods try
    }
    let SwiftValue::Str(field) = &args[0].value else {
        return Err(type_error("value(forHTTPHeaderField:) expects a String"));
    };
    let pairs = header_pairs(&request_field(&recv, "allHTTPHeaderFields")?)?;
    let result = header_lookup(&pairs, field)
        .map(SwiftValue::Str)
        .unwrap_or(SwiftValue::Nil);
    Ok(Some(Outcome {
        result,
        receiver: recv,
    }))
}

fn with_request_headers(
    recv: &SwiftValue,
    pairs: Vec<(String, String)>,
) -> Result<SwiftValue, StdError> {
    let SwiftValue::Struct(o) = recv else {
        return Err(type_error("expected URLRequest"));
    };
    let mut obj = (**o).clone();
    obj.set("allHTTPHeaderFields", headers_value(pairs));
    Ok(SwiftValue::Struct(Rc::new(obj)))
}

// ===========================================================================
// URLResponse / HTTPURLResponse
// ===========================================================================

fn url_response_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut url = None;
    let mut mime = SwiftValue::Nil;
    let mut length = SwiftValue::int(-1);
    let mut encoding = SwiftValue::Nil;
    for arg in &args {
        match arg.label.as_deref() {
            Some("url") => {
                url_string(&arg.value)?;
                url = Some(arg.value.clone());
            }
            Some("mimeType") => mime = arg.value.clone(),
            Some("expectedContentLength") => length = arg.value.clone(),
            Some("textEncodingName") => encoding = arg.value.clone(),
            _ => {
                return Err(type_error(
                    "URLResponse(url:mimeType:expectedContentLength:textEncodingName:)",
                ))
            }
        }
    }
    let Some(url) = url else {
        return Err(type_error("URLResponse requires a url:"));
    };
    Ok(SwiftValue::Struct(Rc::new(StructObj {
        type_name: "URLResponse".into(),
        fields: vec![
            ("url".into(), url),
            ("mimeType".into(), mime),
            ("expectedContentLength".into(), length),
            ("textEncodingName".into(), encoding),
        ],
    })))
}

/// Build an HTTPURLResponse struct value (also used by the URLSession layer).
pub(crate) fn http_url_response_value(
    url: SwiftValue,
    status: i128,
    headers: Vec<(String, String)>,
) -> SwiftValue {
    // Derive the URLResponse-level views from the headers, like Foundation.
    let mime = header_lookup(&headers, "Content-Type")
        .map(|ct| SwiftValue::Str(ct.split(';').next().unwrap_or_default().trim().to_string()))
        .unwrap_or(SwiftValue::Nil);
    let length = header_lookup(&headers, "Content-Length")
        .and_then(|v| v.trim().parse::<i128>().ok())
        .map(SwiftValue::int)
        .unwrap_or(SwiftValue::int(-1));
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "HTTPURLResponse".into(),
        fields: vec![
            ("url".into(), url),
            ("statusCode".into(), SwiftValue::int(status)),
            ("allHeaderFields".into(), headers_value(headers)),
            ("mimeType".into(), mime),
            ("expectedContentLength".into(), length),
            ("textEncodingName".into(), SwiftValue::Nil),
        ],
    }))
}

fn http_url_response_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut url = None;
    let mut status = None;
    let mut headers = Vec::new();
    for arg in &args {
        match arg.label.as_deref() {
            Some("url") => {
                url_string(&arg.value)?;
                url = Some(arg.value.clone());
            }
            Some("statusCode") => {
                status = match &arg.value {
                    SwiftValue::Int(i) => Some(i.raw),
                    _ => return Err(type_error("statusCode must be an Int")),
                }
            }
            // Accepted and ignored (Foundation ignores it too).
            Some("httpVersion") => {}
            Some("headerFields") => headers = header_pairs(&arg.value)?,
            _ => {
                return Err(type_error(
                    "HTTPURLResponse(url:statusCode:httpVersion:headerFields:)",
                ))
            }
        }
    }
    let (Some(url), Some(status)) = (url, status) else {
        return Err(type_error("HTTPURLResponse requires url: and statusCode:"));
    };
    Ok(http_url_response_value(url, status, headers))
}

fn response_field(recv: &SwiftValue, name: &str) -> StdResult {
    match recv {
        SwiftValue::Struct(o)
            if o.type_name == "URLResponse" || o.type_name == "HTTPURLResponse" =>
        {
            Ok(o.get(name).cloned().unwrap_or(SwiftValue::Nil))
        }
        _ => Err(type_error(format!("{name} expects a URLResponse"))),
    }
}

fn response_url(recv: SwiftValue) -> StdResult {
    response_field(&recv, "url")
}

fn response_mime_type(recv: SwiftValue) -> StdResult {
    response_field(&recv, "mimeType")
}

fn response_expected_content_length(recv: SwiftValue) -> StdResult {
    response_field(&recv, "expectedContentLength")
}

fn response_text_encoding_name(recv: SwiftValue) -> StdResult {
    response_field(&recv, "textEncodingName")
}

/// Foundation derives `suggestedFilename` from the URL's last path component,
/// falling back to `"Unknown"`.
fn response_suggested_filename(recv: SwiftValue) -> StdResult {
    let url = response_field(&recv, "url")?;
    let name = url_string(&url)
        .ok()
        .and_then(|s| {
            let path = s.split(['?', '#']).next().unwrap_or("");
            let last = path.trim_end_matches('/').rsplit('/').next()?;
            let last = last.trim();
            // Scheme/host remnants (e.g. "example.com" from "https://example.com")
            // still count as no path component.
            if last.is_empty() || last.contains(':') || path.matches('/').nth(2).is_none() {
                None
            } else {
                Some(last.to_string())
            }
        })
        .unwrap_or_else(|| "Unknown".to_string());
    Ok(SwiftValue::Str(name))
}

fn response_description(recv: SwiftValue) -> StdResult {
    let url = response_field(&recv, "url")?;
    let url = url_string(&url).unwrap_or_default();
    match &recv {
        SwiftValue::Struct(o) if o.type_name == "HTTPURLResponse" => {
            let status = match o.get("statusCode") {
                Some(SwiftValue::Int(i)) => i.raw,
                _ => 0,
            };
            Ok(SwiftValue::Str(format!("<HTTPURLResponse {url} {status}>")))
        }
        _ => Ok(SwiftValue::Str(format!("<URLResponse {url}>"))),
    }
}

fn http_response_status_code(recv: SwiftValue) -> StdResult {
    response_field(&recv, "statusCode")
}

fn http_response_all_header_fields(recv: SwiftValue) -> StdResult {
    let v = response_field(&recv, "allHeaderFields")?;
    Ok(match v {
        SwiftValue::Nil => SwiftValue::Dict(Rc::new(Vec::new())),
        other => other,
    })
}

fn http_response_value_for_field(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    if args.len() != 1 || args[0].label.as_deref() != Some("forHTTPHeaderField") {
        return Ok(None);
    }
    let SwiftValue::Str(field) = &args[0].value else {
        return Err(type_error("value(forHTTPHeaderField:) expects a String"));
    };
    let pairs = header_pairs(&response_field(&recv, "allHeaderFields")?)?;
    let result = header_lookup(&pairs, field)
        .map(SwiftValue::Str)
        .unwrap_or(SwiftValue::Nil);
    Ok(Some(Outcome {
        result,
        receiver: recv,
    }))
}

/// `HTTPURLResponse.localizedString(forStatusCode:)` — the RFC reason phrase,
/// lowercased like Foundation's.
fn http_response_localized_string(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if args.len() != 1 || args[0].label.as_deref() != Some("forStatusCode") {
        return Err(type_error("localizedString expects (forStatusCode: Int)"));
    }
    let SwiftValue::Int(code) = &args[0].value else {
        return Err(type_error("forStatusCode must be an Int"));
    };
    Ok(SwiftValue::Str(status_phrase(code.raw).to_string()))
}

fn status_phrase(code: i128) -> &'static str {
    match code {
        100 => "continue",
        101 => "switching protocols",
        200 => "no error",
        201 => "created",
        202 => "accepted",
        203 => "non-authoritative information",
        204 => "no content",
        205 => "reset content",
        206 => "partial content",
        300 => "multiple choices",
        301 => "moved permanently",
        302 => "found",
        303 => "see other",
        304 => "not modified",
        305 => "use proxy",
        307 => "temporary redirect",
        308 => "permanent redirect",
        400 => "bad request",
        401 => "unauthorized",
        402 => "payment required",
        403 => "forbidden",
        404 => "not found",
        405 => "method not allowed",
        406 => "unacceptable",
        407 => "proxy authentication required",
        408 => "request timed out",
        409 => "conflict",
        410 => "no longer exists",
        411 => "length required",
        412 => "precondition failed",
        413 => "request too large",
        414 => "requested URL too long",
        415 => "unsupported media type",
        416 => "requested range not satisfiable",
        417 => "expectation failed",
        429 => "too many requests",
        500 => "internal server error",
        501 => "unimplemented",
        502 => "bad gateway",
        503 => "service unavailable",
        504 => "gateway timed out",
        505 => "unsupported version",
        c if (100..200).contains(&c) => "informational",
        c if (200..300).contains(&c) => "success",
        c if (300..400).contains(&c) => "redirected",
        c if (400..500).contains(&c) => "client error",
        c if (500..600).contains(&c) => "server error",
        _ => "server error",
    }
}

// ===========================================================================
// URLError
// ===========================================================================

fn url_error_code_value(case: &str) -> SwiftValue {
    SwiftValue::Enum(Rc::new(tswift_core::EnumObj {
        type_name: "URLError.Code".into(),
        case: case.into(),
        payload: Vec::new(),
    }))
}

/// Build a URLError struct value (also used by the URLSession layer).
pub(crate) fn url_error_value(case: &str, failing_url: SwiftValue) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "URLError".into(),
        fields: vec![
            ("code".into(), url_error_code_value(case)),
            ("failingURL".into(), failing_url),
        ],
    }))
}

fn url_error_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if args.is_empty() {
        return Err(type_error("URLError requires a code"));
    }
    let SwiftValue::Enum(e) = &args[0].value else {
        return Err(type_error("URLError expects a URLError.Code"));
    };
    if e.type_name != "URLError.Code" {
        return Err(type_error(format!(
            "URLError expects a URLError.Code, got {}",
            e.type_name
        )));
    }
    if args[0].label.is_some() {
        return Err(type_error("URLError code takes no label"));
    }
    if args.len() > 1 {
        return Err(type_error("unsupported URLError arguments"));
    }
    Ok(url_error_value(&e.case, SwiftValue::Nil))
}

fn url_error_field(recv: &SwiftValue, name: &str) -> StdResult {
    match recv {
        SwiftValue::Struct(o) if o.type_name == "URLError" => {
            Ok(o.get(name).cloned().unwrap_or(SwiftValue::Nil))
        }
        _ => Err(type_error(format!("{name} expects URLError"))),
    }
}

fn url_error_case(recv: &SwiftValue) -> Result<String, StdError> {
    match url_error_field(recv, "code")? {
        SwiftValue::Enum(e) => Ok(e.case.clone()),
        _ => Err(type_error("malformed URLError")),
    }
}

fn url_error_code(recv: SwiftValue) -> StdResult {
    url_error_field(&recv, "code")
}

fn url_error_error_code(recv: SwiftValue) -> StdResult {
    let case = url_error_case(&recv)?;
    let raw = URL_ERROR_CODES
        .iter()
        .find(|(name, _)| *name == case)
        .map(|(_, raw)| *raw)
        .unwrap_or(-1);
    Ok(SwiftValue::int(raw))
}

fn url_error_failing_url(recv: SwiftValue) -> StdResult {
    url_error_field(&recv, "failingURL")
}

fn url_error_localized_description(recv: SwiftValue) -> StdResult {
    let case = url_error_case(&recv)?;
    let text = match case.as_str() {
        "cancelled" => "cancelled",
        "badURL" => "The URL is not valid.",
        "timedOut" => "The request timed out.",
        "unsupportedURL" => "The URL is not supported.",
        "cannotFindHost" => "The server could not be found.",
        "cannotConnectToHost" => "Could not connect to the server.",
        "networkConnectionLost" => "The network connection was lost.",
        "dnsLookupFailed" => "The server could not be found.",
        "httpTooManyRedirects" => "Too many redirects.",
        "resourceUnavailable" => "The requested resource is unavailable.",
        "notConnectedToInternet" => "The Internet connection appears to be offline.",
        "badServerResponse" => "The server gave an invalid response.",
        "userCancelledAuthentication" => "Authentication was cancelled.",
        "userAuthenticationRequired" => "Authentication is required.",
        "zeroByteResource" => "The requested resource is empty.",
        "cannotDecodeRawData" => "The data could not be decoded.",
        "cannotDecodeContentData" => "The content could not be decoded.",
        "cannotParseResponse" => "The server response could not be parsed.",
        "dataNotAllowed" => "Cellular data is not allowed.",
        "secureConnectionFailed" => "A secure connection could not be established.",
        _ => "The operation could not be completed.",
    };
    Ok(SwiftValue::Str(text.to_string()))
}

fn url_error_hash_value(recv: SwiftValue) -> StdResult {
    let case = url_error_case(&recv)?;
    Ok(SwiftValue::int(crate::fnv1a_hash(case.as_bytes())))
}

/// `URLError.failureURLString` — the failing URL's `String` form. Honestly
/// `nil`: the runtime has no `userInfo` dictionary to source
/// `NSURLErrorFailingURLStringErrorKey` from independently of `failingURL`,
/// and Foundation itself deprecated this key in favor of `failingURL`
/// (macOS 15.4+); we don't model the redundant string form.
fn url_error_failure_url_string(recv: SwiftValue) -> StdResult {
    url_error_field(&recv, "failingURL")?; // validate receiver type
    Ok(SwiftValue::Nil)
}

/// `URLError.failureURLPeerTrust` — the `SecTrust` from a failed TLS
/// handshake. Honestly `nil`: the runtime has no TLS/SecTrust stack.
fn url_error_failure_url_peer_trust(recv: SwiftValue) -> StdResult {
    url_error_field(&recv, "code")?; // validate receiver type
    Ok(SwiftValue::Nil)
}

/// `URLError.networkUnavailableReason` — why the network was unreachable
/// (cellular/expensive/constrained). Honestly `nil`: the runtime has no
/// reachability or network-constraint model.
fn url_error_network_unavailable_reason(recv: SwiftValue) -> StdResult {
    url_error_field(&recv, "code")?; // validate receiver type
    Ok(SwiftValue::Nil)
}

/// `URLError.backgroundTaskCancelledReason` — why a background
/// `URLSessionTask` was cancelled. Honestly `nil`: the runtime has no
/// background-session model.
fn url_error_background_task_cancelled_reason(recv: SwiftValue) -> StdResult {
    url_error_field(&recv, "code")?; // validate receiver type
    Ok(SwiftValue::Nil)
}

/// `URLError.downloadTaskResumeData` / `.uploadTaskResumeData` — resume data
/// from a cancelled download/upload task. Honestly `nil`: the runtime has no
/// `URLSessionDownloadTask`/upload-task resume-data model.
fn url_error_download_task_resume_data(recv: SwiftValue) -> StdResult {
    url_error_field(&recv, "code")?; // validate receiver type
    Ok(SwiftValue::Nil)
}

fn url_error_upload_task_resume_data(recv: SwiftValue) -> StdResult {
    url_error_field(&recv, "code")?; // validate receiver type
    Ok(SwiftValue::Nil)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::url::url_value;

    #[test]
    fn header_lookup_is_case_insensitive() {
        let pairs = vec![("Content-Type".to_string(), "text/plain".to_string())];
        assert_eq!(
            header_lookup(&pairs, "content-type").as_deref(),
            Some("text/plain")
        );
        assert_eq!(header_lookup(&pairs, "Accept"), None);
    }

    #[test]
    fn http_response_derives_mime_and_length_from_headers() {
        let resp = http_url_response_value(
            url_value("https://example.com/a".into()),
            200,
            vec![
                (
                    "Content-Type".to_string(),
                    "application/json; charset=utf-8".to_string(),
                ),
                ("Content-Length".to_string(), "42".to_string()),
            ],
        );
        assert_eq!(
            response_mime_type(resp.clone()).unwrap(),
            SwiftValue::Str("application/json".into())
        );
        assert_eq!(
            response_expected_content_length(resp.clone()).unwrap(),
            SwiftValue::int(42)
        );
        assert_eq!(
            http_response_status_code(resp).unwrap(),
            SwiftValue::int(200)
        );
    }

    #[test]
    fn http_response_without_length_header_reports_unknown() {
        let resp = http_url_response_value(url_value("https://example.com".into()), 204, vec![]);
        assert_eq!(
            response_expected_content_length(resp.clone()).unwrap(),
            SwiftValue::int(-1)
        );
        assert_eq!(response_mime_type(resp).unwrap(), SwiftValue::Nil);
    }

    #[test]
    fn suggested_filename_uses_last_path_component_or_unknown() {
        let resp = http_url_response_value(
            url_value("https://example.com/files/report.pdf?x=1".into()),
            200,
            vec![],
        );
        assert_eq!(
            response_suggested_filename(resp).unwrap(),
            SwiftValue::Str("report.pdf".into())
        );
        let bare = http_url_response_value(url_value("https://example.com".into()), 200, vec![]);
        assert_eq!(
            response_suggested_filename(bare).unwrap(),
            SwiftValue::Str("Unknown".into())
        );
    }

    #[test]
    fn status_phrases_match_foundation_wording() {
        assert_eq!(status_phrase(200), "no error");
        assert_eq!(status_phrase(404), "not found");
        assert_eq!(status_phrase(503), "service unavailable");
        assert_eq!(status_phrase(299), "success");
        assert_eq!(status_phrase(999), "server error");
    }

    #[test]
    fn url_error_maps_cases_to_ns_error_codes() {
        let err = url_error_value("timedOut", SwiftValue::Nil);
        assert_eq!(
            url_error_error_code(err.clone()).unwrap(),
            SwiftValue::int(-1001)
        );
        assert_eq!(url_error_case(&err).unwrap(), "timedOut".to_string());
    }

    #[test]
    fn request_value_defaults_match_foundation() {
        let req = url_request_value(
            url_value("https://example.com".into()),
            60.0,
            SwiftValue::Nil,
            SwiftValue::Str("GET".into()),
            SwiftValue::Nil,
        );
        assert_eq!(
            url_request_http_method(req.clone()).unwrap(),
            SwiftValue::Str("GET".into())
        );
        assert_eq!(
            url_request_timeout_interval(req.clone()).unwrap(),
            SwiftValue::Double(60.0)
        );
        assert_eq!(url_request_http_body(req).unwrap(), SwiftValue::Nil);
    }
}
