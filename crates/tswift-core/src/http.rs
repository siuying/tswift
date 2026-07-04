//! The HTTP transport seam behind `URLSession`.
//!
//! The interpreter is a cooperative single-threaded executor (ADR-0005), so
//! the seam stays **synchronous** from the interpreter's viewpoint.
//! Embeddings choose a backend:
//!
//! - golden fixtures / tests: [`MockHttpTransport`] (deterministic, offline);
//! - the CLI: a real blocking HTTPS client;
//! - native embeds (`tswift-ffi`): a host-registered handler (which may itself
//!   be backed by the platform's real `URLSession`);
//! - wasm: a synchronous imported host function.
//!
//! ADR-0011 extends the seam with [`HttpTransport::start`] /
//! [`HttpTransport::next_event`] / [`HttpTransport::cancel`] to enable
//! delegate callbacks, mid-flight cancellation, and progress. The one-shot
//! [`HttpTransport::perform`] is the required method for backward
//! compatibility; `start`/`next_event`/`cancel` have provided defaults backed
//! by [`SingleShotEvents`]. Backends that need native streaming override all
//! three (M2+).
//!
//! No transport configured means `URLSession` reports an unsupported-feature
//! interpreter error rather than a Swift-visible `URLError`, so scripts cannot
//! confuse "sandboxed" with "network down".

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::VecDeque;

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

/// Opaque per-transport in-flight request id, returned by
/// [`HttpTransport::start`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HttpTaskHandle(pub u64);

/// An event produced by an in-flight HTTP request.
///
/// Event-order contract: exactly one [`HttpEvent::Response`] arrives first,
/// then zero or more [`HttpEvent::Chunk`] events, then exactly one terminal
/// event ([`HttpEvent::Done`] or [`HttpEvent::Failed`]). The interpreter loop
/// will enforce this contract in M3+; until then, malformed sequences from a
/// misbehaving transport may surface as unexpected behaviour rather than a
/// tidy `badServerResponse`.
#[derive(Debug, Clone, PartialEq)]
pub enum HttpEvent {
    /// Status line + headers arrived. Exactly one, first.
    Response {
        status: i64,
        headers: Vec<(String, String)>,
    },
    /// One body fragment. Zero or more.
    Chunk(Vec<u8>),
    /// Terminal: success.
    Done,
    /// Terminal: failure carrying a `URLError.Code` case name.
    Failed { code: String, message: String },
}

impl HttpEvent {
    /// Returns `true` for terminal events ([`Done`][HttpEvent::Done] or
    /// [`Failed`][HttpEvent::Failed]).
    pub fn is_terminal(&self) -> bool {
        matches!(self, HttpEvent::Done | HttpEvent::Failed { .. })
    }
}

/// Adapts a one-shot `Result<HttpResponse, HttpError>` into the canonical
/// `Response → Chunk(body)? → Done` / `Failed` event sequence, so simple
/// backends can implement only [`HttpTransport::perform`] and get streaming
/// semantics for free via the default `start`/`next_event`/`cancel` impls.
pub struct SingleShotEvents {
    events: VecDeque<HttpEvent>,
}

impl SingleShotEvents {
    /// Build the event sequence from a one-shot outcome.
    ///
    /// - `Ok(resp)` → `Response` → `Chunk(body)` (if non-empty) → `Done`
    /// - `Err(Failed)` → `Failed`
    /// - `Err(Unavailable)` → propagated, not wrapped (callers check first)
    pub fn from_outcome(outcome: Result<HttpResponse, HttpError>) -> Self {
        let mut events = VecDeque::new();
        match outcome {
            Ok(resp) => {
                events.push_back(HttpEvent::Response {
                    status: resp.status,
                    headers: resp.headers,
                });
                if !resp.body.is_empty() {
                    events.push_back(HttpEvent::Chunk(resp.body));
                }
                events.push_back(HttpEvent::Done);
            }
            Err(HttpError::Failed { code, message }) => {
                events.push_back(HttpEvent::Failed { code, message });
            }
            Err(HttpError::Unavailable) => {
                // Callers of from_outcome should guard against Unavailable;
                // if it slips through, surface a generic failure.
                events.push_back(HttpEvent::Failed {
                    code: "unsupported".into(),
                    message: "HTTP transport unavailable".into(),
                });
            }
        }
        Self { events }
    }

    /// Pop and return the next event. After all events are consumed, returns
    /// a `Failed(badServerResponse)` sentinel (callers should not call past a
    /// terminal event).
    pub fn next_event(&mut self) -> HttpEvent {
        self.events
            .pop_front()
            .unwrap_or_else(|| HttpEvent::Failed {
                code: "badServerResponse".into(),
                message: "SingleShotEvents consumed past terminal event".into(),
            })
    }

    /// Returns `true` when all events have been consumed.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Thread-local storage backing the provided default `start`/`next_event`/
// `cancel` implementations. Each thread gets its own ID counter and pending
// map, which is safe because the interpreter is single-threaded (ADR-0005).
// ---------------------------------------------------------------------------

thread_local! {
    static DEFAULT_NEXT_ID: Cell<u64> = const { Cell::new(1) };
    static DEFAULT_PENDING: RefCell<HashMap<u64, SingleShotEvents>> =
        RefCell::new(HashMap::new());
}

/// A synchronous HTTP backend. See the module docs for the embedding matrix.
///
/// ## Required method
///
/// [`perform`][HttpTransport::perform] is the only required method. The
/// `start` / `next_event` / `cancel` trio has provided defaults backed by a
/// thread-local [`SingleShotEvents`] queue so existing one-shot backends keep
/// compiling unchanged. Backends that need native streaming override all
/// three (M2+).
pub trait HttpTransport {
    /// Perform `req`, blocking until a response or failure is available.
    fn perform(&mut self, req: &HttpRequest) -> Result<HttpResponse, HttpError>;

    /// Start `req` and return an opaque handle.
    ///
    /// The default implementation calls [`perform`][HttpTransport::perform]
    /// eagerly and wraps the result in a [`SingleShotEvents`] queue stored in
    /// a thread-local, so `next_event` / `cancel` work without any override.
    fn start(&mut self, req: &HttpRequest) -> Result<HttpTaskHandle, HttpError> {
        let outcome = self.perform(req);
        if let Err(HttpError::Unavailable) = &outcome {
            return Err(HttpError::Unavailable);
        }
        let id = DEFAULT_NEXT_ID.with(|n| {
            let v = n.get();
            n.set(v + 1);
            v
        });
        DEFAULT_PENDING.with(|p| {
            p.borrow_mut()
                .insert(id, SingleShotEvents::from_outcome(outcome));
        });
        Ok(HttpTaskHandle(id))
    }

    /// Block until the next event for `h`.
    ///
    /// After a terminal event ([`HttpEvent::Done`] or [`HttpEvent::Failed`])
    /// is returned, `h` is considered dead — calling `next_event` again is
    /// safe but returns a `Failed(badServerResponse)` sentinel.
    ///
    /// **Invariant (caller):** every handle returned by [`start`][HttpTransport::start]
    /// must be consumed to a terminal event OR explicitly [`cancel`][HttpTransport::cancel]led
    /// (and then polled once to drain the `Failed{cancelled}` terminal) to
    /// avoid leaking entries in the backing store.
    fn next_event(&mut self, h: HttpTaskHandle) -> HttpEvent {
        DEFAULT_PENDING.with(|p| {
            let mut map = p.borrow_mut();
            let event = map.get_mut(&h.0).map(|sse| sse.next_event());
            match event {
                Some(e) => {
                    if e.is_terminal() {
                        map.remove(&h.0);
                    }
                    e
                }
                None => HttpEvent::Failed {
                    code: "badServerResponse".into(),
                    message: "unknown or exhausted task handle".into(),
                },
            }
        })
    }

    /// Best-effort abort. After this returns, the **next** call to
    /// `next_event(h)` will return `Failed { code: "cancelled" }` as a
    /// terminal event; any call after that returns the
    /// `Failed(badServerResponse)` dead-handle sentinel.
    ///
    /// Callers must poll `next_event` once after `cancel` to drain the
    /// terminal event and release the backing entry (see the invariant on
    /// [`next_event`][HttpTransport::next_event]).
    ///
    /// Backends that support native streaming should override this to signal
    /// cancellation to the transport layer (M2+).
    fn cancel(&mut self, h: HttpTaskHandle) {
        DEFAULT_PENDING.with(|p| {
            // Replace any remaining events with a single terminal
            // Failed{cancelled} so the next next_event(h) call honours the
            // cancel contract rather than returning the badServerResponse
            // sentinel.  next_event's terminal-cleanup removes the entry.
            p.borrow_mut().insert(
                h.0,
                SingleShotEvents::from_outcome(Err(HttpError::failed(
                    "cancelled",
                    "request cancelled",
                ))),
            );
        });
    }
}

// ---------------------------------------------------------------------------
// Event wire codec
// ---------------------------------------------------------------------------

/// Encode an [`HttpEvent`] as the event-stream JSON wire format shared by the
/// FFI and wasm host transports.
///
/// ```text
/// {"event":"response","status":200,"headers":[["Content-Type","text/plain"]]}
/// {"event":"chunk","bodyBase64":"aGk="}
/// {"event":"done"}
/// {"event":"error","code":"timedOut","message":"..."}
/// ```
pub fn encode_event_json(event: &HttpEvent) -> String {
    use crate::result_json::escape;
    match event {
        HttpEvent::Response { status, headers } => {
            let mut s = format!(
                "{{\"event\":\"response\",\"status\":{},\"headers\":[",
                status
            );
            for (i, (k, v)) in headers.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                s.push_str(&format!("[\"{}\",\"{}\"]", escape(k), escape(v)));
            }
            s.push_str("]}");
            s
        }
        HttpEvent::Chunk(bytes) => {
            format!(
                "{{\"event\":\"chunk\",\"bodyBase64\":\"{}\"}}",
                crate::base64::encode(bytes)
            )
        }
        HttpEvent::Done => "{\"event\":\"done\"}".to_string(),
        HttpEvent::Failed { code, message } => {
            format!(
                "{{\"event\":\"error\",\"code\":\"{}\",\"message\":\"{}\"}}",
                escape(code),
                escape(message)
            )
        }
    }
}

/// Decode an event-stream JSON object into an [`HttpEvent`].
///
/// Unknown `"event"` values and structural errors map to
/// `Err(HttpError::Failed { code: "badServerResponse", … })` rather than
/// panicking, so the interpreter loop can apply the event-order lenience rule.
pub fn decode_event_json(text: &str) -> Result<HttpEvent, HttpError> {
    use crate::json::{self, Json};
    let malformed = |m: &str| HttpError::failed("badServerResponse", m);
    let root = json::parse(text).map_err(|e| malformed(&format!("event JSON parse error: {e}")))?;
    let event_type = match root.get("event") {
        Some(Json::Str(s)) => s.clone(),
        _ => return Err(malformed("event JSON missing string `event` field")),
    };
    match event_type.as_str() {
        "response" => {
            let status = match root.get("status") {
                Some(Json::Int(s)) => *s,
                _ => return Err(malformed("response event missing integer `status`")),
            };
            let mut headers = Vec::new();
            if let Some(Json::Array(pairs)) = root.get("headers") {
                for pair in pairs {
                    let Json::Array(kv) = pair else {
                        return Err(malformed(
                            "response event headers must be [[k,v],...] pairs",
                        ));
                    };
                    let (Some(Json::Str(k)), Some(Json::Str(v))) = (kv.first(), kv.get(1)) else {
                        return Err(malformed(
                            "response event headers must be [[k,v],...] pairs",
                        ));
                    };
                    headers.push((k.clone(), v.clone()));
                }
            }
            Ok(HttpEvent::Response { status, headers })
        }
        "chunk" => {
            let body = match root.get("bodyBase64") {
                Some(Json::Str(b64)) => crate::base64::decode(b64)
                    .ok_or_else(|| malformed("chunk event bodyBase64 is not valid base64"))?,
                _ => return Err(malformed("chunk event missing string `bodyBase64`")),
            };
            Ok(HttpEvent::Chunk(body))
        }
        "done" => Ok(HttpEvent::Done),
        "error" => {
            let code = match root.get("code") {
                Some(Json::Str(s)) => s.clone(),
                _ => "unknown".to_string(),
            };
            let message = match root.get("message") {
                Some(Json::Str(s)) => s.clone(),
                _ => "transport error".to_string(),
            };
            Ok(HttpEvent::Failed { code, message })
        }
        other => Err(malformed(&format!("unknown event type: {other}"))),
    }
}

// ---------------------------------------------------------------------------
// Wire codec for requests / one-shot responses (unchanged from ADR-0010)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// MockHttpTransport
// ---------------------------------------------------------------------------

/// One scripted route of a [`MockHttpTransport`].
#[derive(Debug, Clone)]
pub struct MockRoute {
    /// HTTP method to match (case-insensitive).
    pub method: String,
    /// Absolute URL to match exactly.
    pub url: String,
    /// The scripted one-shot outcome for a matching request.
    pub outcome: Result<HttpResponse, HttpError>,
}

/// A scripted chunked route for [`MockHttpTransport`] that delivers a response
/// as a sequence of body fragments, optionally ending with a mid-stream error.
///
/// Add via [`MockHttpTransport::with_chunked_routes`]. Chunked routes are
/// checked before regular [`MockRoute`]s.
#[derive(Debug, Clone)]
pub struct MockChunkedRoute {
    /// HTTP method to match (case-insensitive).
    pub method: String,
    /// Absolute URL to match exactly.
    pub url: String,
    /// HTTP status code for the `Response` event.
    pub status: i64,
    /// Response headers for the `Response` event.
    pub headers: Vec<(String, String)>,
    /// Body fragments delivered as successive `Chunk` events.
    pub chunks: Vec<Vec<u8>>,
    /// If `Some((code, message))`, a `Failed` event is emitted after all
    /// chunks; otherwise a `Done` event closes the stream.
    pub fail_after_chunks: Option<(String, String)>,
}

/// A deterministic scripted transport for tests and golden fixtures.
///
/// Requests are answered from a route table; anything unrouted fails like an
/// unknown host, so fixtures cannot silently hit the real network.
///
/// ## Streaming support
///
/// [`start`][HttpTransport::start] / [`next_event`][HttpTransport::next_event]
/// / [`cancel`][HttpTransport::cancel] are overridden to support chunked
/// routes (added via [`with_chunked_routes`][MockHttpTransport::with_chunked_routes]).
/// Regular routes are delivered as a single-chunk sequence via
/// [`SingleShotEvents`].
#[derive(Debug, Default)]
pub struct MockHttpTransport {
    routes: Vec<MockRoute>,
    chunked_routes: Vec<MockChunkedRoute>,
    /// Monotonically increasing ID for in-flight handles.
    next_id: u64,
    /// Pending event queues keyed by handle ID.
    pending: HashMap<u64, VecDeque<HttpEvent>>,
}

impl MockHttpTransport {
    /// A transport answering from `routes`.
    pub fn new(routes: Vec<MockRoute>) -> MockHttpTransport {
        MockHttpTransport {
            routes,
            ..Default::default()
        }
    }

    /// Add chunked routes to this transport (checked before regular routes).
    pub fn with_chunked_routes(mut self, chunked: Vec<MockChunkedRoute>) -> MockHttpTransport {
        self.chunked_routes = chunked;
        self
    }

    fn build_chunked_events(cr: &MockChunkedRoute) -> VecDeque<HttpEvent> {
        let mut q = VecDeque::new();
        q.push_back(HttpEvent::Response {
            status: cr.status,
            headers: cr.headers.clone(),
        });
        for chunk in &cr.chunks {
            q.push_back(HttpEvent::Chunk(chunk.clone()));
        }
        match &cr.fail_after_chunks {
            Some((code, message)) => q.push_back(HttpEvent::Failed {
                code: code.clone(),
                message: message.clone(),
            }),
            None => q.push_back(HttpEvent::Done),
        }
        q
    }

    fn pop_event(&mut self, h: HttpTaskHandle) -> HttpEvent {
        let event = self.pending.get_mut(&h.0).and_then(|q| q.pop_front());
        match event {
            Some(e) => {
                if e.is_terminal() {
                    self.pending.remove(&h.0);
                }
                e
            }
            None => HttpEvent::Failed {
                code: "badServerResponse".into(),
                message: "unknown or exhausted task handle".into(),
            },
        }
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

    fn start(&mut self, req: &HttpRequest) -> Result<HttpTaskHandle, HttpError> {
        self.next_id += 1;
        let id = self.next_id;

        let events = if let Some(cr) = self
            .chunked_routes
            .iter()
            .find(|r| r.method.eq_ignore_ascii_case(&req.method) && r.url == req.url)
        {
            Self::build_chunked_events(cr)
        } else {
            let outcome = self.perform(req);
            if let Err(HttpError::Unavailable) = &outcome {
                return Err(HttpError::Unavailable);
            }
            let mut sse = SingleShotEvents::from_outcome(outcome);
            let mut q = VecDeque::new();
            loop {
                let e = sse.next_event();
                let terminal = e.is_terminal();
                q.push_back(e);
                if terminal {
                    break;
                }
            }
            q
        };

        self.pending.insert(id, events);
        Ok(HttpTaskHandle(id))
    }

    fn next_event(&mut self, h: HttpTaskHandle) -> HttpEvent {
        self.pop_event(h)
    }

    fn cancel(&mut self, h: HttpTaskHandle) {
        // Replace any remaining events with a single terminal Failed{cancelled}
        // so the next next_event(h) call honours the cancel contract.  The
        // entry is cleaned up when that terminal event is consumed.
        let mut q = VecDeque::new();
        q.push_back(HttpEvent::Failed {
            code: "cancelled".into(),
            message: "request cancelled".into(),
        });
        self.pending.insert(h.0, q);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn get(url: &str) -> HttpRequest {
        HttpRequest {
            url: url.to_string(),
            method: "GET".to_string(),
            headers: Vec::new(),
            body: None,
            timeout_seconds: 60.0,
        }
    }

    fn ok_resp(status: i64, body: &[u8]) -> HttpResponse {
        HttpResponse {
            status,
            headers: vec![("Content-Type".into(), "text/plain".into())],
            body: body.to_vec(),
        }
    }

    fn collect_events(transport: &mut dyn HttpTransport, h: HttpTaskHandle) -> Vec<HttpEvent> {
        let mut events = Vec::new();
        loop {
            let e = transport.next_event(h);
            let terminal = e.is_terminal();
            events.push(e);
            if terminal {
                break;
            }
        }
        events
    }

    // -----------------------------------------------------------------------
    // SingleShotEvents
    // -----------------------------------------------------------------------

    #[test]
    fn single_shot_events_success_with_body_yields_response_chunk_done() {
        let mut sse = SingleShotEvents::from_outcome(Ok(ok_resp(200, b"hello")));
        assert_eq!(
            sse.next_event(),
            HttpEvent::Response {
                status: 200,
                headers: vec![("Content-Type".into(), "text/plain".into())]
            }
        );
        assert_eq!(sse.next_event(), HttpEvent::Chunk(b"hello".to_vec()));
        assert_eq!(sse.next_event(), HttpEvent::Done);
        assert!(sse.is_empty());
    }

    #[test]
    fn single_shot_events_empty_body_skips_chunk_event() {
        let mut sse = SingleShotEvents::from_outcome(Ok(HttpResponse {
            status: 204,
            headers: Vec::new(),
            body: Vec::new(),
        }));
        assert!(matches!(
            sse.next_event(),
            HttpEvent::Response { status: 204, .. }
        ));
        assert_eq!(sse.next_event(), HttpEvent::Done);
        assert!(sse.is_empty());
    }

    #[test]
    fn single_shot_events_failure_yields_single_failed_event() {
        let mut sse = SingleShotEvents::from_outcome(Err(HttpError::failed("timedOut", "timeout")));
        assert_eq!(
            sse.next_event(),
            HttpEvent::Failed {
                code: "timedOut".into(),
                message: "timeout".into()
            }
        );
        assert!(sse.is_empty());
    }

    #[test]
    fn single_shot_events_past_terminal_returns_sentinel() {
        let mut sse = SingleShotEvents::from_outcome(Ok(HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: Vec::new(),
        }));
        // Consume Response + Done
        let _ = sse.next_event();
        let _ = sse.next_event();
        // Calling past terminal returns a sentinel, not a panic
        let sentinel = sse.next_event();
        assert!(matches!(sentinel, HttpEvent::Failed { code, .. } if code == "badServerResponse"));
    }

    // -----------------------------------------------------------------------
    // Event JSON codec
    // -----------------------------------------------------------------------

    #[test]
    fn encode_decode_response_event_round_trips() {
        let event = HttpEvent::Response {
            status: 200,
            headers: vec![("Content-Type".into(), "text/plain".into())],
        };
        let json = encode_event_json(&event);
        let decoded = decode_event_json(&json).unwrap();
        assert_eq!(decoded, event);
    }

    #[test]
    fn encode_decode_chunk_event_round_trips() {
        let event = HttpEvent::Chunk(b"hello world".to_vec());
        let json = encode_event_json(&event);
        let decoded = decode_event_json(&json).unwrap();
        assert_eq!(decoded, event);
    }

    #[test]
    fn encode_decode_done_event_round_trips() {
        let json = encode_event_json(&HttpEvent::Done);
        let decoded = decode_event_json(&json).unwrap();
        assert_eq!(decoded, HttpEvent::Done);
    }

    #[test]
    fn encode_decode_failed_event_round_trips() {
        let event = HttpEvent::Failed {
            code: "timedOut".into(),
            message: "request timed out".into(),
        };
        let json = encode_event_json(&event);
        let decoded = decode_event_json(&json).unwrap();
        assert_eq!(decoded, event);
    }

    #[test]
    fn encode_response_event_uses_correct_field_names() {
        let json = encode_event_json(&HttpEvent::Response {
            status: 404,
            headers: vec![("X-Foo".into(), "bar".into())],
        });
        assert!(json.contains("\"event\":\"response\""));
        assert!(json.contains("\"status\":404"));
        assert!(json.contains("\"X-Foo\""));
    }

    #[test]
    fn encode_chunk_event_uses_base64_body() {
        let json = encode_event_json(&HttpEvent::Chunk(b"hi".to_vec()));
        assert!(json.contains("\"event\":\"chunk\""));
        assert!(json.contains("\"bodyBase64\":\"aGk=\""));
    }

    #[test]
    fn encode_failed_event_uses_error_key() {
        let json = encode_event_json(&HttpEvent::Failed {
            code: "cancelled".into(),
            message: "user cancelled".into(),
        });
        assert!(json.contains("\"event\":\"error\""));
        assert!(json.contains("\"code\":\"cancelled\""));
    }

    #[test]
    fn decode_event_json_rejects_missing_event_field() {
        let err = decode_event_json(r#"{"status": 200}"#).unwrap_err();
        assert!(matches!(err, HttpError::Failed { code, .. } if code == "badServerResponse"));
    }

    #[test]
    fn decode_event_json_rejects_unknown_event_type() {
        let err = decode_event_json(r#"{"event": "stream"}"#).unwrap_err();
        assert!(matches!(err, HttpError::Failed { code, .. } if code == "badServerResponse"));
    }

    #[test]
    fn decode_event_json_rejects_invalid_json() {
        let err = decode_event_json("not json").unwrap_err();
        assert!(matches!(err, HttpError::Failed { code, .. } if code == "badServerResponse"));
    }

    #[test]
    fn decode_response_event_without_headers_field_succeeds() {
        // headers array is optional (defaults to empty)
        let decoded = decode_event_json(r#"{"event":"response","status":204}"#).unwrap();
        assert_eq!(
            decoded,
            HttpEvent::Response {
                status: 204,
                headers: Vec::new()
            }
        );
    }

    #[test]
    fn decode_error_event_defaults_code_and_message_when_absent() {
        let decoded = decode_event_json(r#"{"event":"error"}"#).unwrap();
        assert!(matches!(decoded, HttpEvent::Failed { .. }));
    }

    // -----------------------------------------------------------------------
    // MockHttpTransport — one-shot paths (perform + default start/next/cancel)
    // -----------------------------------------------------------------------

    #[test]
    fn mock_answers_matching_route() {
        let mut mock = MockHttpTransport::new(vec![MockRoute {
            method: "get".into(),
            url: "https://example.com/a".into(),
            outcome: Ok(ok_resp(200, b"hi")),
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

    #[test]
    fn mock_start_next_event_delivers_response_chunk_done_for_regular_route() {
        let mut mock = MockHttpTransport::new(vec![MockRoute {
            method: "GET".into(),
            url: "https://example.com/x".into(),
            outcome: Ok(ok_resp(200, b"body")),
        }]);
        let h = mock.start(&get("https://example.com/x")).unwrap();
        let events = collect_events(&mut mock, h);
        assert_eq!(events.len(), 3);
        assert!(matches!(
            &events[0],
            HttpEvent::Response { status: 200, .. }
        ));
        assert_eq!(events[1], HttpEvent::Chunk(b"body".to_vec()));
        assert_eq!(events[2], HttpEvent::Done);
    }

    #[test]
    fn mock_start_next_event_delivers_failed_for_error_route() {
        let mut mock = MockHttpTransport::new(vec![MockRoute {
            method: "GET".into(),
            url: "https://down.example.com/".into(),
            outcome: Err(HttpError::failed("timedOut", "scripted")),
        }]);
        let h = mock.start(&get("https://down.example.com/")).unwrap();
        let events = collect_events(&mut mock, h);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], HttpEvent::Failed { code, .. } if code == "timedOut"));
    }

    #[test]
    fn mock_cancel_yields_cancelled_terminal_then_sentinel() {
        let mut mock = MockHttpTransport::new(vec![MockRoute {
            method: "GET".into(),
            url: "https://example.com/y".into(),
            outcome: Ok(ok_resp(200, b"data")),
        }]);
        let h = mock.start(&get("https://example.com/y")).unwrap();
        mock.cancel(h);
        // First post-cancel poll must return Failed{cancelled}, not badServerResponse
        let e = mock.next_event(h);
        assert!(
            matches!(&e, HttpEvent::Failed { code, .. } if code == "cancelled"),
            "expected Failed{{cancelled}}, got {e:?}"
        );
        // After the terminal is consumed the handle is dead — sentinel
        let sentinel = mock.next_event(h);
        assert!(
            matches!(sentinel, HttpEvent::Failed { ref code, .. } if code == "badServerResponse"),
            "expected badServerResponse sentinel, got {sentinel:?}"
        );
    }

    #[test]
    fn mock_cancel_mid_stream_yields_cancelled_then_sentinel() {
        // Cancel after consuming only the Response event (before Done)
        let mut mock = MockHttpTransport::new(vec![MockRoute {
            method: "GET".into(),
            url: "https://example.com/z".into(),
            outcome: Ok(ok_resp(200, b"body")),
        }]);
        let h = mock.start(&get("https://example.com/z")).unwrap();
        let first = mock.next_event(h); // Response
        assert!(matches!(first, HttpEvent::Response { .. }));
        mock.cancel(h);
        let e = mock.next_event(h);
        assert!(
            matches!(&e, HttpEvent::Failed { code, .. } if code == "cancelled"),
            "expected Failed{{cancelled}} after mid-stream cancel, got {e:?}"
        );
        let sentinel = mock.next_event(h);
        assert!(
            matches!(sentinel, HttpEvent::Failed { ref code, .. } if code == "badServerResponse"),
            "expected dead-handle sentinel, got {sentinel:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Default start / next_event / cancel implementations (perform-only backend)
    //
    // A transport that only overrides `perform` gets the thread-local
    // SingleShotEvents queue for free via the trait defaults.  These tests
    // pin that behaviour without using MockHttpTransport.
    // -----------------------------------------------------------------------

    /// Minimal perform-only transport — does NOT override start/next_event/cancel.
    struct PerformOnlyTransport {
        /// Returns the stored outcome once; subsequent calls return an error.
        outcome: Option<Result<HttpResponse, HttpError>>,
    }

    impl PerformOnlyTransport {
        fn success(status: i64, body: &[u8]) -> Self {
            Self {
                outcome: Some(Ok(ok_resp(status, body))),
            }
        }
        fn failure(code: &str, message: &str) -> Self {
            Self {
                outcome: Some(Err(HttpError::failed(code, message))),
            }
        }
    }

    impl HttpTransport for PerformOnlyTransport {
        fn perform(&mut self, _req: &HttpRequest) -> Result<HttpResponse, HttpError> {
            self.outcome
                .take()
                .unwrap_or_else(|| Err(HttpError::failed("cannotFindHost", "exhausted")))
        }
        // start / next_event / cancel are NOT overridden — uses trait defaults
    }

    #[test]
    fn default_start_next_event_delivers_response_chunk_done() {
        let mut t = PerformOnlyTransport::success(200, b"hello");
        let h = t.start(&get("https://example.com/")).unwrap();
        let events = collect_events(&mut t, h);
        assert_eq!(
            events.len(),
            3,
            "expected Response+Chunk+Done, got {events:?}"
        );
        assert!(matches!(
            &events[0],
            HttpEvent::Response { status: 200, .. }
        ));
        assert_eq!(events[1], HttpEvent::Chunk(b"hello".to_vec()));
        assert_eq!(events[2], HttpEvent::Done);
    }

    #[test]
    fn default_start_next_event_delivers_failed_on_error() {
        let mut t = PerformOnlyTransport::failure("timedOut", "scripted timeout");
        let h = t.start(&get("https://example.com/")).unwrap();
        let events = collect_events(&mut t, h);
        assert_eq!(
            events.len(),
            1,
            "expected single Failed event, got {events:?}"
        );
        assert!(
            matches!(&events[0], HttpEvent::Failed { code, .. } if code == "timedOut"),
            "expected Failed{{timedOut}}, got {:?}",
            events[0]
        );
    }

    #[test]
    fn default_cancel_yields_cancelled_terminal_then_sentinel() {
        let mut t = PerformOnlyTransport::success(200, b"data");
        let h = t.start(&get("https://example.com/")).unwrap();
        t.cancel(h);
        // First post-cancel poll must return Failed{cancelled} per cancel contract
        let e = t.next_event(h);
        assert!(
            matches!(&e, HttpEvent::Failed { code, .. } if code == "cancelled"),
            "expected Failed{{cancelled}} from default cancel, got {e:?}"
        );
        // After terminal is consumed the handle is dead — sentinel
        let sentinel = t.next_event(h);
        assert!(
            matches!(sentinel, HttpEvent::Failed { ref code, .. } if code == "badServerResponse"),
            "expected dead-handle sentinel, got {sentinel:?}"
        );
    }

    #[test]
    fn default_past_terminal_returns_sentinel() {
        let mut t = PerformOnlyTransport::success(200, b"");
        let h = t.start(&get("https://example.com/")).unwrap();
        // Drain to terminal (Response + Done for empty body)
        let _ = collect_events(&mut t, h);
        // Any further poll returns the dead-handle sentinel
        let sentinel = t.next_event(h);
        assert!(
            matches!(sentinel, HttpEvent::Failed { ref code, .. } if code == "badServerResponse"),
            "expected sentinel after terminal consumed, got {sentinel:?}"
        );
    }

    // -----------------------------------------------------------------------
    // MockHttpTransport — chunked routes
    // -----------------------------------------------------------------------

    #[test]
    fn mock_chunked_route_delivers_multiple_chunks_then_done() {
        let mut mock = MockHttpTransport::default().with_chunked_routes(vec![MockChunkedRoute {
            method: "GET".into(),
            url: "https://stream.example.com/data".into(),
            status: 200,
            headers: vec![("Content-Type".into(), "application/octet-stream".into())],
            chunks: vec![b"chunk1".to_vec(), b"chunk2".to_vec(), b"chunk3".to_vec()],
            fail_after_chunks: None,
        }]);
        let h = mock.start(&get("https://stream.example.com/data")).unwrap();
        let events = collect_events(&mut mock, h);
        assert_eq!(events.len(), 5); // Response + 3 chunks + Done
        assert!(matches!(
            &events[0],
            HttpEvent::Response { status: 200, .. }
        ));
        assert_eq!(events[1], HttpEvent::Chunk(b"chunk1".to_vec()));
        assert_eq!(events[2], HttpEvent::Chunk(b"chunk2".to_vec()));
        assert_eq!(events[3], HttpEvent::Chunk(b"chunk3".to_vec()));
        assert_eq!(events[4], HttpEvent::Done);
    }

    #[test]
    fn mock_chunked_route_mid_stream_failure_ends_with_failed() {
        let mut mock = MockHttpTransport::default().with_chunked_routes(vec![MockChunkedRoute {
            method: "GET".into(),
            url: "https://stream.example.com/flaky".into(),
            status: 200,
            headers: Vec::new(),
            chunks: vec![b"part1".to_vec(), b"part2".to_vec()],
            fail_after_chunks: Some(("networkConnectionLost".into(), "dropped".into())),
        }]);
        let h = mock
            .start(&get("https://stream.example.com/flaky"))
            .unwrap();
        let events = collect_events(&mut mock, h);
        assert_eq!(events.len(), 4); // Response + 2 chunks + Failed
        assert!(matches!(&events[0], HttpEvent::Response { .. }));
        assert_eq!(events[1], HttpEvent::Chunk(b"part1".to_vec()));
        assert_eq!(events[2], HttpEvent::Chunk(b"part2".to_vec()));
        assert!(
            matches!(&events[3], HttpEvent::Failed { code, .. } if code == "networkConnectionLost")
        );
    }

    #[test]
    fn mock_chunked_route_zero_chunks_delivers_response_then_done() {
        let mut mock = MockHttpTransport::default().with_chunked_routes(vec![MockChunkedRoute {
            method: "GET".into(),
            url: "https://example.com/empty-stream".into(),
            status: 204,
            headers: Vec::new(),
            chunks: Vec::new(),
            fail_after_chunks: None,
        }]);
        let h = mock
            .start(&get("https://example.com/empty-stream"))
            .unwrap();
        let events = collect_events(&mut mock, h);
        assert_eq!(events.len(), 2); // Response + Done
        assert!(matches!(
            &events[0],
            HttpEvent::Response { status: 204, .. }
        ));
        assert_eq!(events[1], HttpEvent::Done);
    }

    #[test]
    fn mock_chunked_route_takes_priority_over_regular_route() {
        // Same URL in both tables — chunked wins
        let url = "https://example.com/shared";
        let mut mock = MockHttpTransport::new(vec![MockRoute {
            method: "GET".into(),
            url: url.into(),
            outcome: Ok(ok_resp(500, b"should-not-see")),
        }])
        .with_chunked_routes(vec![MockChunkedRoute {
            method: "GET".into(),
            url: url.into(),
            status: 200,
            headers: Vec::new(),
            chunks: vec![b"streamed".to_vec()],
            fail_after_chunks: None,
        }]);
        let h = mock.start(&get(url)).unwrap();
        let events = collect_events(&mut mock, h);
        assert!(matches!(
            &events[0],
            HttpEvent::Response { status: 200, .. }
        ));
        assert_eq!(events[1], HttpEvent::Chunk(b"streamed".to_vec()));
    }

    #[test]
    fn mock_two_in_flight_handles_are_independent() {
        // Start two requests concurrently; their events don't cross.
        let mut mock = MockHttpTransport::new(vec![
            MockRoute {
                method: "GET".into(),
                url: "https://a.example/".into(),
                outcome: Ok(ok_resp(200, b"aaa")),
            },
            MockRoute {
                method: "GET".into(),
                url: "https://b.example/".into(),
                outcome: Ok(ok_resp(201, b"bbb")),
            },
        ]);
        let ha = mock.start(&get("https://a.example/")).unwrap();
        let hb = mock.start(&get("https://b.example/")).unwrap();
        // Interleave polling
        let ea0 = mock.next_event(ha); // Response(200)
        let eb0 = mock.next_event(hb); // Response(201)
        let ea1 = mock.next_event(ha); // Chunk(aaa)
        let eb1 = mock.next_event(hb); // Chunk(bbb)
        let ea2 = mock.next_event(ha); // Done
        let eb2 = mock.next_event(hb); // Done
        assert!(matches!(ea0, HttpEvent::Response { status: 200, .. }));
        assert!(matches!(eb0, HttpEvent::Response { status: 201, .. }));
        assert_eq!(ea1, HttpEvent::Chunk(b"aaa".to_vec()));
        assert_eq!(eb1, HttpEvent::Chunk(b"bbb".to_vec()));
        assert_eq!(ea2, HttpEvent::Done);
        assert_eq!(eb2, HttpEvent::Done);
    }

    // -----------------------------------------------------------------------
    // Request/response wire codec (unchanged from ADR-0010)
    // -----------------------------------------------------------------------

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
}
