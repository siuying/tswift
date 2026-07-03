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
