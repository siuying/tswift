//! `URLSession` / `URLSessionConfiguration` on the [`tswift_core::http`]
//! transport seam.
//!
//! The interpreter's executor is cooperative and single-threaded (ADR-0005),
//! so `data(from:)` / `data(for:)` perform one synchronous
//! [`StdContext::perform_http`] call: `async` adds no suspension here.
//! Transport failures surface as thrown `URLError` values; a missing
//! transport (sandboxed embedding) is an interpreter error instead, so
//! scripts cannot mistake "no network capability" for a network failure.

use std::rc::Rc;

use tswift_core::{
    Arg, BuiltinReceiver, EvalError, HttpError, HttpRequest, LabeledMethodEntry, Outcome,
    StdContext, StdError, StdResult, StructObj, SwiftValue,
};

use crate::network::{http_url_response_value, url_error_value};
use crate::type_error;
use crate::url::url_string;
use crate::{data_bytes, data_value};

/// Register `URLSession` and `URLSessionConfiguration` on `interp`.
pub(crate) fn install(interp: &mut tswift_core::Interpreter<'_>) {
    // ---- URLSessionConfiguration ----
    interp.register_static_value("URLSessionConfiguration", "default", configuration_value());
    interp.register_static_value(
        "URLSessionConfiguration",
        "ephemeral",
        configuration_value(),
    );

    // ---- URLSession ----
    interp.register_static_value("URLSession", "shared", session_value(configuration_value()));
    interp.register_free_fn("URLSession", session_init);
    interp.register_property(
        BuiltinReceiver::URLSession,
        "configuration",
        session_configuration,
    );
    interp.register_labeled_intrinsic(
        BuiltinReceiver::URLSession,
        "data",
        LabeledMethodEntry {
            mutating: false,
            func: session_data,
        },
    );
    interp.register_labeled_intrinsic(
        BuiltinReceiver::URLSession,
        "upload",
        LabeledMethodEntry {
            mutating: false,
            func: session_upload,
        },
    );
}

/// The default/ephemeral configuration value (the runtime has no URL cache or
/// cookie storage, so the two presets coincide).
fn configuration_value() -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "URLSessionConfiguration".into(),
        fields: vec![
            ("timeoutIntervalForRequest".into(), SwiftValue::Double(60.0)),
            (
                "timeoutIntervalForResource".into(),
                SwiftValue::Double(604_800.0),
            ),
            ("httpAdditionalHeaders".into(), SwiftValue::Nil),
            ("allowsCellularAccess".into(), SwiftValue::Bool(true)),
            ("waitsForConnectivity".into(), SwiftValue::Bool(false)),
        ],
    }))
}

fn session_value(configuration: SwiftValue) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "URLSession".into(),
        fields: vec![("configuration".into(), configuration)],
    }))
}

fn session_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if args.len() != 1 || args[0].label.as_deref() != Some("configuration") {
        return Err(type_error("URLSession(configuration:) expects one label"));
    }
    match &args[0].value {
        SwiftValue::Struct(o) if o.type_name == "URLSessionConfiguration" => {
            Ok(session_value(args[0].value.clone()))
        }
        _ => Err(type_error(
            "URLSession(configuration:) expects a URLSessionConfiguration",
        )),
    }
}

fn session_configuration(recv: SwiftValue) -> StdResult {
    match &recv {
        SwiftValue::Struct(o) if o.type_name == "URLSession" => Ok(o
            .get("configuration")
            .cloned()
            .unwrap_or_else(configuration_value)),
        _ => Err(type_error("configuration expects URLSession")),
    }
}

/// The session's request timeout (`configuration.timeoutIntervalForRequest`).
fn session_timeout(recv: &SwiftValue) -> f64 {
    let SwiftValue::Struct(o) = recv else {
        return 60.0;
    };
    let Some(SwiftValue::Struct(config)) = o.get("configuration") else {
        return 60.0;
    };
    match config.get("timeoutIntervalForRequest") {
        Some(SwiftValue::Double(d)) => *d,
        Some(SwiftValue::Int(i)) => i.raw as f64,
        _ => 60.0,
    }
}

/// Lower a `URLRequest` struct value into a transport [`HttpRequest`].
fn lower_request(request: &SwiftValue) -> Result<HttpRequest, StdError> {
    let SwiftValue::Struct(o) = request else {
        return Err(type_error("expected URLRequest"));
    };
    if o.type_name != "URLRequest" {
        return Err(type_error(format!(
            "expected URLRequest, got {}",
            o.type_name
        )));
    }
    let url = match o.get("url") {
        Some(u) => url_string(u)?,
        None => return Err(type_error("URLRequest has no url")),
    };
    let method = match o.get("httpMethod") {
        Some(SwiftValue::Str(m)) => m.clone(),
        _ => "GET".to_string(),
    };
    let mut headers = Vec::new();
    if let Some(SwiftValue::Dict(pairs)) = o.get("allHTTPHeaderFields") {
        for (k, v) in pairs.iter() {
            if let (SwiftValue::Str(k), SwiftValue::Str(v)) = (k, v) {
                headers.push((k.clone(), v.clone()));
            }
        }
    }
    let body = match o.get("httpBody") {
        Some(SwiftValue::Nil) | None => None,
        Some(data) => Some(data_bytes(data)?),
    };
    let timeout_seconds = match o.get("timeoutInterval") {
        Some(SwiftValue::Double(d)) => *d,
        Some(SwiftValue::Int(i)) => i.raw as f64,
        _ => 60.0,
    };
    Ok(HttpRequest {
        url,
        method,
        headers,
        body,
        timeout_seconds,
    })
}

/// Perform `req` and produce the `(Data, URLResponse)` tuple, translating
/// failures per the module contract.
fn perform(ctx: &mut dyn StdContext, req: HttpRequest) -> StdResult {
    match ctx.perform_http(&req) {
        Ok(resp) => {
            let response = http_url_response_value(
                crate::url::url_value(req.url.clone()),
                i128::from(resp.status),
                resp.headers,
            );
            Ok(SwiftValue::Tuple(
                vec![data_value(resp.body), response],
                vec![None, None],
            ))
        }
        Err(HttpError::Failed { code, message: _ }) => Err(StdError::Throw(url_error_value(
            &code,
            crate::url::url_value(req.url),
        ))),
        Err(HttpError::Unavailable) => Err(StdError::Error(EvalError::Unsupported(
            "URLSession needs a network transport; this embedding has none configured".into(),
        ))),
    }
}

/// `data(from: URL)` / `data(for: URLRequest)`.
fn session_data(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    if args.len() != 1 {
        return Ok(None);
    }
    let req = match args[0].label.as_deref() {
        Some("from") => HttpRequest {
            url: url_string(&args[0].value)?,
            method: "GET".to_string(),
            headers: Vec::new(),
            body: None,
            timeout_seconds: session_timeout(&recv),
        },
        Some("for") => lower_request(&args[0].value)?,
        _ => return Ok(None),
    };
    let result = perform(ctx, req)?;
    Ok(Some(Outcome {
        result,
        receiver: recv,
    }))
}

/// `upload(for: URLRequest, from: Data)` — the request performed with `body`.
fn session_upload(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    if args.len() != 2
        || args[0].label.as_deref() != Some("for")
        || args[1].label.as_deref() != Some("from")
    {
        return Ok(None);
    }
    let mut req = lower_request(&args[0].value)?;
    req.body = Some(data_bytes(&args[1].value)?);
    let result = perform(ctx, req)?;
    Ok(Some(Outcome {
        result,
        receiver: recv,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::url::url_value;
    use tswift_core::{HttpResponse, MockHttpTransport, MockRoute};

    /// A minimal `StdContext` carrying only an HTTP transport.
    struct HttpOnlyCtx {
        transport: MockHttpTransport,
        out: Vec<u8>,
    }

    impl StdContext for HttpOnlyCtx {
        fn call_closure(&mut self, _id: usize, _args: Vec<SwiftValue>) -> StdResult {
            Err(type_error("no closures in this test context"))
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            &mut self.out
        }
        fn perform_http(
            &mut self,
            req: &HttpRequest,
        ) -> Result<HttpResponse, tswift_core::HttpError> {
            use tswift_core::HttpTransport;
            self.transport.perform(req)
        }
    }

    fn request_value(url: &str) -> SwiftValue {
        crate::network::url_request_value(
            url_value(url.into()),
            60.0,
            SwiftValue::Nil,
            SwiftValue::Str("GET".into()),
            SwiftValue::Nil,
        )
    }

    #[test]
    fn lower_request_carries_url_method_and_timeout() {
        let req = lower_request(&request_value("https://example.com/a")).unwrap();
        assert_eq!(req.url, "https://example.com/a");
        assert_eq!(req.method, "GET");
        assert_eq!(req.timeout_seconds, 60.0);
        assert_eq!(req.body, None);
        assert!(req.headers.is_empty());
    }

    #[test]
    fn data_from_url_returns_data_and_response_tuple() {
        let mut ctx = HttpOnlyCtx {
            transport: MockHttpTransport::new(vec![MockRoute {
                method: "GET".into(),
                url: "https://example.com/hello".into(),
                outcome: Ok(HttpResponse {
                    status: 200,
                    headers: vec![("Content-Type".into(), "text/plain".into())],
                    body: b"hello".to_vec(),
                }),
            }]),
            out: Vec::new(),
        };
        let session = session_value(configuration_value());
        let outcome = session_data(
            &mut ctx,
            session,
            vec![Arg {
                label: Some("from".into()),
                value: url_value("https://example.com/hello".into()),
            }],
        )
        .unwrap()
        .unwrap();
        let SwiftValue::Tuple(items, _) = outcome.result else {
            panic!("expected (Data, URLResponse) tuple");
        };
        assert_eq!(data_bytes(&items[0]).unwrap(), b"hello".to_vec());
        let SwiftValue::Struct(resp) = &items[1] else {
            panic!("expected HTTPURLResponse struct");
        };
        assert_eq!(resp.type_name, "HTTPURLResponse");
        assert_eq!(resp.get("statusCode"), Some(&SwiftValue::int(200)));
    }

    #[test]
    fn transport_failure_throws_a_url_error() {
        let mut ctx = HttpOnlyCtx {
            transport: MockHttpTransport::default(),
            out: Vec::new(),
        };
        let session = session_value(configuration_value());
        let err = session_data(
            &mut ctx,
            session,
            vec![Arg {
                label: Some("from".into()),
                value: url_value("https://nowhere.invalid/".into()),
            }],
        )
        .unwrap_err();
        let StdError::Throw(SwiftValue::Struct(o)) = err else {
            panic!("expected a thrown URLError");
        };
        assert_eq!(o.type_name, "URLError");
    }

    #[test]
    fn missing_transport_is_an_interpreter_error_not_a_throw() {
        struct NoNet(Vec<u8>);
        impl StdContext for NoNet {
            fn call_closure(&mut self, _id: usize, _args: Vec<SwiftValue>) -> StdResult {
                Err(type_error("unused"))
            }
            fn out(&mut self) -> &mut dyn std::io::Write {
                &mut self.0
            }
        }
        let mut ctx = NoNet(Vec::new());
        let err = session_data(
            &mut ctx,
            session_value(configuration_value()),
            vec![Arg {
                label: Some("from".into()),
                value: url_value("https://example.com/".into()),
            }],
        )
        .unwrap_err();
        assert!(matches!(err, StdError::Error(EvalError::Unsupported(_))));
    }

    #[test]
    fn upload_overrides_the_request_body() {
        let mut ctx = HttpOnlyCtx {
            transport: MockHttpTransport::new(vec![MockRoute {
                method: "GET".into(),
                url: "https://example.com/up".into(),
                outcome: Ok(HttpResponse {
                    status: 201,
                    headers: Vec::new(),
                    body: Vec::new(),
                }),
            }]),
            out: Vec::new(),
        };
        let outcome = session_upload(
            &mut ctx,
            session_value(configuration_value()),
            vec![
                Arg {
                    label: Some("for".into()),
                    value: request_value("https://example.com/up"),
                },
                Arg {
                    label: Some("from".into()),
                    value: data_value(b"payload".to_vec()),
                },
            ],
        )
        .unwrap()
        .unwrap();
        let SwiftValue::Tuple(items, _) = outcome.result else {
            panic!("expected tuple");
        };
        let SwiftValue::Struct(resp) = &items[1] else {
            panic!("expected response struct");
        };
        assert_eq!(resp.get("statusCode"), Some(&SwiftValue::int(201)));
    }
}
