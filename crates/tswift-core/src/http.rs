//! The HTTP transport seam behind `URLSession`.
//!
//! The interpreter is a cooperative single-threaded executor (ADR-0005), so
//! the seam is deliberately **synchronous**: `URLSession.data` performs one
//! blocking [`HttpTransport::perform`] call from the interpreter's viewpoint.
//! Each embedding chooses its backend:
//!
//! - golden fixtures / tests: [`MockHttpTransport`] (deterministic, offline);
//! - the CLI: a real blocking HTTPS client;
//! - native embeds (`tswift-ffi`): a host-registered handler (which may itself
//!   be backed by the platform's real `URLSession`);
//! - wasm: a synchronous imported host function.
//!
//! No transport configured means `URLSession` reports an unsupported-feature
//! interpreter error rather than a Swift-visible `URLError`, so scripts cannot
//! confuse "sandboxed" with "network down".

/// One HTTP request handed to a transport: everything `URLRequest` carries,
/// already lowered to plain Rust types.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpRequest {
    /// Absolute URL string.
    pub url: String,
    /// HTTP method (`GET`, `POST`, ...).
    pub method: String,
    /// Header fields in insertion order. Field names are case-insensitive.
    pub headers: Vec<(String, String)>,
    /// Request body bytes, if any.
    pub body: Option<Vec<u8>>,
    /// Request timeout in seconds (`URLRequest.timeoutInterval`).
    pub timeout_seconds: f64,
}

/// One HTTP response handed back by a transport.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpResponse {
    /// HTTP status code (200, 404, ...).
    pub status: i64,
    /// Response header fields in wire order.
    pub headers: Vec<(String, String)>,
    /// Response body bytes.
    pub body: Vec<u8>,
}

/// Why a transport could not produce a response.
#[derive(Debug, Clone, PartialEq)]
pub enum HttpError {
    /// No transport is configured in this embedding (sandboxed run). Surfaced
    /// as an interpreter error, not a Swift `URLError`.
    Unavailable,
    /// A transport-level failure, carrying a `URLError.Code` case name
    /// (`"cannotFindHost"`, `"timedOut"`, ...) plus a human-readable message.
    Failed { code: String, message: String },
}

impl HttpError {
    /// A transport failure with `URLError.Code` case `code`.
    pub fn failed(code: impl Into<String>, message: impl Into<String>) -> HttpError {
        HttpError::Failed {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// Serialize a transport request as the host-boundary request JSON
/// (`{"url","method","timeoutSeconds","headers":[[k,v]...],"bodyBase64"?}`),
/// the shared wire contract of the FFI and wasm host transports.
pub fn encode_request_json(req: &HttpRequest) -> String {
    use crate::result_json::escape;
    let mut s = format!(
        "{{\"url\":\"{}\",\"method\":\"{}\",\"timeoutSeconds\":{},\"headers\":[",
        escape(&req.url),
        escape(&req.method),
        req.timeout_seconds
    );
    for (i, (k, v)) in req.headers.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!("[\"{}\",\"{}\"]", escape(k), escape(v)));
    }
    s.push(']');
    if let Some(body) = &req.body {
        s.push_str(&format!(
            ",\"bodyBase64\":\"{}\"",
            crate::base64::encode(body)
        ));
    }
    s.push('}');
    s
}

/// Parse a host's response JSON (`{"status","headers":[[k,v]...],
/// "bodyBase64"?}` or `{"error":"<URLError.Code case>","message"?}`) into a
/// transport response or failure — the inverse of [`encode_request_json`]'s
/// wire contract.
pub fn decode_response_json(text: &str) -> Result<HttpResponse, HttpError> {
    use crate::json::{self, Json};
    let malformed = |m: &str| HttpError::failed("badServerResponse", m);
    let root = json::parse(text)
        .map_err(|e| malformed(&format!("host HTTP response is not valid JSON: {e}")))?;
    if let Some(Json::Str(code)) = root.get("error") {
        let message = match root.get("message") {
            Some(Json::Str(m)) => m.clone(),
            _ => "host HTTP handler reported a failure".to_string(),
        };
        return Err(HttpError::failed(code.clone(), message));
    }
    let status = match root.get("status") {
        Some(Json::Int(s)) => *s,
        _ => return Err(malformed("host HTTP response has no integer `status`")),
    };
    let mut headers = Vec::new();
    if let Some(Json::Array(pairs)) = root.get("headers") {
        for pair in pairs {
            let Json::Array(kv) = pair else {
                return Err(malformed("host HTTP response headers must be [k, v] pairs"));
            };
            let (Some(Json::Str(k)), Some(Json::Str(v))) = (kv.first(), kv.get(1)) else {
                return Err(malformed("host HTTP response headers must be [k, v] pairs"));
            };
            headers.push((k.clone(), v.clone()));
        }
    }
    let body = match root.get("bodyBase64") {
        Some(Json::Str(b64)) => crate::base64::decode(b64)
            .ok_or_else(|| malformed("host HTTP response bodyBase64 is not valid base64"))?,
        _ => Vec::new(),
    };
    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

/// A synchronous HTTP backend. See the module docs for the embedding matrix.
pub trait HttpTransport {
    /// Perform `req`, blocking until a response or failure is available.
    fn perform(&mut self, req: &HttpRequest) -> Result<HttpResponse, HttpError>;
}

/// One scripted route of a [`MockHttpTransport`].
#[derive(Debug, Clone)]
pub struct MockRoute {
    /// HTTP method to match (case-insensitive).
    pub method: String,
    /// Absolute URL to match exactly.
    pub url: String,
    /// The scripted outcome for a matching request.
    pub outcome: Result<HttpResponse, HttpError>,
}

/// A deterministic scripted transport for tests and golden fixtures: requests
/// are answered from a route table; anything unrouted fails like an unknown
/// host, so fixtures cannot silently hit the real network.
#[derive(Debug, Default)]
pub struct MockHttpTransport {
    routes: Vec<MockRoute>,
}

impl MockHttpTransport {
    /// A transport answering from `routes`.
    pub fn new(routes: Vec<MockRoute>) -> MockHttpTransport {
        MockHttpTransport { routes }
    }
}

impl HttpTransport for MockHttpTransport {
    fn perform(&mut self, req: &HttpRequest) -> Result<HttpResponse, HttpError> {
        self.routes
            .iter()
            .find(|r| r.method.eq_ignore_ascii_case(&req.method) && r.url == req.url)
            .map(|r| r.outcome.clone())
            .unwrap_or_else(|| {
                Err(HttpError::failed(
                    "cannotFindHost",
                    format!("no mock route for {} {}", req.method, req.url),
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(url: &str) -> HttpRequest {
        HttpRequest {
            url: url.to_string(),
            method: "GET".to_string(),
            headers: Vec::new(),
            body: None,
            timeout_seconds: 60.0,
        }
    }

    #[test]
    fn mock_answers_matching_route() {
        let mut mock = MockHttpTransport::new(vec![MockRoute {
            method: "get".into(),
            url: "https://example.com/a".into(),
            outcome: Ok(HttpResponse {
                status: 200,
                headers: vec![("Content-Type".into(), "text/plain".into())],
                body: b"hi".to_vec(),
            }),
        }]);
        let resp = mock.perform(&get("https://example.com/a")).unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, b"hi");
    }

    #[test]
    fn mock_fails_unrouted_requests_like_unknown_host() {
        let mut mock = MockHttpTransport::default();
        let err = mock.perform(&get("https://example.com/b")).unwrap_err();
        match err {
            HttpError::Failed { code, .. } => assert_eq!(code, "cannotFindHost"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn wire_codec_round_trips_request_and_response() {
        let req = HttpRequest {
            url: "https://example.com/a".into(),
            method: "POST".into(),
            headers: vec![("Content-Type".into(), "text/plain".into())],
            body: Some(b"hi".to_vec()),
            timeout_seconds: 30.0,
        };
        let json = encode_request_json(&req);
        let root = crate::json::parse(&json).unwrap();
        assert_eq!(
            root.get("url"),
            Some(&crate::json::Json::Str("https://example.com/a".into()))
        );
        assert_eq!(
            root.get("bodyBase64"),
            Some(&crate::json::Json::Str("aGk=".into()))
        );

        let ok = decode_response_json(
            r#"{"status": 200, "headers": [["Content-Type", "text/plain"]], "bodyBase64": "aGk="}"#,
        )
        .unwrap();
        assert_eq!(ok.status, 200);
        assert_eq!(ok.body, b"hi");
        let err = decode_response_json(r#"{"error": "timedOut"}"#).unwrap_err();
        assert!(matches!(err, HttpError::Failed { code, .. } if code == "timedOut"));
        assert!(decode_response_json("not json").is_err());
    }

    #[test]
    fn mock_replays_scripted_failures() {
        let mut mock = MockHttpTransport::new(vec![MockRoute {
            method: "GET".into(),
            url: "https://down.example.com/".into(),
            outcome: Err(HttpError::failed("timedOut", "scripted timeout")),
        }]);
        let err = mock.perform(&get("https://down.example.com/")).unwrap_err();
        assert_eq!(err, HttpError::failed("timedOut", "scripted timeout"));
    }
}
