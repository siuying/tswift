//! The real HTTPS transport behind `tswift run --allow-network`.
//!
//! One blocking `ureq` (rustls) call per [`HttpTransport::perform`] — a match
//! for the interpreter's synchronous transport seam (see `tswift_core::http`).
//! Non-2xx statuses are responses, not errors, mirroring `URLSession`; only
//! transport-level failures map onto `URLError.Code` case names.
//!
//! **M5 streaming** — [`NetTransport`] overrides
//! [`HttpTransport::start`] / [`HttpTransport::next_event`] /
//! [`HttpTransport::cancel`] to stream the response body in fixed-size
//! chunks (~64 KiB) instead of reading everything at once.
//! `cancel` drops the reader; the next `next_event` call then returns
//! `Failed { code: "cancelled" }` per the seam contract (ADR-0011).

use std::collections::HashMap;
use std::fmt;
use std::io::{self, Read};
use std::time::Duration;

use tswift_core::{HttpError, HttpEvent, HttpRequest, HttpResponse, HttpTaskHandle, HttpTransport};

/// Target chunk size for streaming reads (~64 KiB).
const CHUNK_SIZE: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// Per-request stream state
// ---------------------------------------------------------------------------

/// Internal state of one in-flight streaming request.
///
/// The state machine is:
/// `PendingResponse` → (first `next_event`) → `Reading` → (EOF) → removed
///                                                        → (error) → removed
/// Any state → (`cancel`) → `Cancelled` → (next `next_event`) → removed
enum StreamState {
    /// The underlying HTTP connection succeeded; headers are ready but the
    /// `Response` event has not yet been delivered to the caller.
    PendingResponse {
        status: i64,
        headers: Vec<(String, String)>,
        reader: Box<dyn io::Read>,
    },
    /// `Response` event has been delivered; body chunks are being streamed.
    Reading(Box<dyn io::Read>),
    /// `cancel()` was called; the next `next_event` returns
    /// `Failed { code: "cancelled" }` and removes the entry.
    Cancelled,
}

// ---------------------------------------------------------------------------
// NetTransport
// ---------------------------------------------------------------------------

/// A real network transport; each request builds a per-timeout agent.
pub struct NetTransport {
    next_id: u64,
    pending: HashMap<u64, StreamState>,
}

impl fmt::Debug for NetTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NetTransport")
            .field("next_id", &self.next_id)
            .field("pending_count", &self.pending.len())
            .finish()
    }
}

impl Default for NetTransport {
    fn default() -> Self {
        NetTransport {
            next_id: 0,
            pending: HashMap::new(),
        }
    }
}

impl HttpTransport for NetTransport {
    /// One-shot blocking request; used by callers that don't need events.
    fn perform(&mut self, req: &HttpRequest) -> Result<HttpResponse, HttpError> {
        let agent = build_agent(req.timeout_seconds);
        let ureq_req = build_ureq_request(req)?;
        let response = agent.run(ureq_req).map_err(translate)?;

        let (parts, mut body) = response.into_parts();
        let status = i64::from(parts.status.as_u16());
        let headers = collect_headers(&parts.headers);
        let body_bytes = body
            .read_to_vec()
            .map_err(|e| HttpError::failed("cannotDecodeRawData", e.to_string()))?;

        Ok(HttpResponse {
            status,
            headers,
            body: body_bytes,
        })
    }

    /// Start a request and return a handle for event-driven polling.
    ///
    /// The underlying connection is established synchronously (headers
    /// arrive immediately); body bytes are streamed lazily through
    /// `next_event`.
    fn start(&mut self, req: &HttpRequest) -> Result<HttpTaskHandle, HttpError> {
        let agent = build_agent(req.timeout_seconds);
        let ureq_req = build_ureq_request(req)?;
        let response = agent.run(ureq_req).map_err(translate)?;

        let (parts, body) = response.into_parts();
        let status = i64::from(parts.status.as_u16());
        let headers = collect_headers(&parts.headers);
        let reader: Box<dyn io::Read> = Box::new(body.into_reader());

        self.next_id += 1;
        let id = self.next_id;
        self.pending.insert(
            id,
            StreamState::PendingResponse {
                status,
                headers,
                reader,
            },
        );
        Ok(HttpTaskHandle(id))
    }

    /// Return the next event for an in-flight handle.
    ///
    /// Sequence: `Response` (once) → `Chunk` (zero or more) → `Done` /
    /// `Failed`.  After the terminal event the handle is dead; further calls
    /// return the `Failed(badServerResponse)` sentinel.
    fn next_event(&mut self, h: HttpTaskHandle) -> HttpEvent {
        match self.pending.remove(&h.0) {
            None => HttpEvent::Failed {
                code: "badServerResponse".into(),
                message: "unknown or exhausted task handle".into(),
            },

            Some(StreamState::Cancelled) => HttpEvent::Failed {
                code: "cancelled".into(),
                message: "request cancelled".into(),
            },

            Some(StreamState::PendingResponse {
                status,
                headers,
                reader,
            }) => {
                // Transition to Reading; the caller will get Chunk/Done next.
                self.pending.insert(h.0, StreamState::Reading(reader));
                HttpEvent::Response { status, headers }
            }

            Some(StreamState::Reading(mut reader)) => {
                let mut buf = vec![0u8; CHUNK_SIZE];
                match reader.read(&mut buf) {
                    Ok(0) => {
                        // EOF — entry already removed by the `remove` above.
                        HttpEvent::Done
                    }
                    Ok(n) => {
                        buf.truncate(n);
                        // Re-insert reader for subsequent polls.
                        self.pending.insert(h.0, StreamState::Reading(reader));
                        HttpEvent::Chunk(buf)
                    }
                    Err(e) => {
                        // Entry already removed; translate to URLError code.
                        HttpEvent::Failed {
                            code: translate_io_error(&e).into(),
                            message: e.to_string(),
                        }
                    }
                }
            }
        }
    }

    /// Best-effort abort. The existing reader is dropped; the next
    /// `next_event(h)` call returns `Failed { code: "cancelled" }`.
    fn cancel(&mut self, h: HttpTaskHandle) {
        // Replacing any existing state drops the reader (closing the
        // underlying connection) and marks the handle so the next
        // next_event returns the required terminal event.
        self.pending.insert(h.0, StreamState::Cancelled);
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Build a per-request ureq agent with a global timeout.
fn build_agent(timeout_seconds: f64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs_f64(
            timeout_seconds.clamp(0.001, 86_400.0),
        )))
        .http_status_as_error(false)
        .build()
        .into()
}

/// Build a ureq HTTP request value from our `HttpRequest`.
fn build_ureq_request(req: &HttpRequest) -> Result<ureq::http::Request<Vec<u8>>, HttpError> {
    let mut builder = ureq::http::Request::builder()
        .method(req.method.as_str())
        .uri(&req.url);
    for (name, value) in &req.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder
        .body(req.body.clone().unwrap_or_default())
        .map_err(|e| HttpError::failed("badURL", e.to_string()))
}

/// Collect ureq header map into our wire format.
fn collect_headers(headers: &ureq::http::HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect()
}

/// Map a `ureq` failure onto the closest `URLError.Code` case.
fn translate(e: ureq::Error) -> HttpError {
    let code = match &e {
        ureq::Error::Timeout(_) => "timedOut",
        ureq::Error::HostNotFound => "cannotFindHost",
        ureq::Error::ConnectionFailed => "cannotConnectToHost",
        ureq::Error::BadUri(_) => "badURL",
        ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => "httpTooManyRedirects",
        ureq::Error::Tls(_) | ureq::Error::TlsRequired => "secureConnectionFailed",
        ureq::Error::Protocol(_) => "cannotParseResponse",
        ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => "timedOut",
        ureq::Error::Io(_) => "networkConnectionLost",
        _ => "cannotParseResponse",
    };
    HttpError::failed(code, e.to_string())
}

/// Map an `io::Error` during body streaming to a `URLError.Code` case.
fn translate_io_error(e: &io::Error) -> &'static str {
    match e.kind() {
        io::ErrorKind::TimedOut => "timedOut",
        io::ErrorKind::ConnectionAborted | io::ErrorKind::ConnectionReset => {
            "networkConnectionLost"
        }
        io::ErrorKind::BrokenPipe => "networkConnectionLost",
        _ => "networkConnectionLost",
    }
}

// ---------------------------------------------------------------------------
// Test helpers: inject in-memory readers without a network round-trip
// ---------------------------------------------------------------------------

/// Inject an in-memory stream into the pending table so unit tests can drive
/// `next_event` / `cancel` without any network I/O.
///
/// Returns the handle for the injected stream.
#[cfg(test)]
fn push_in_memory_stream(
    transport: &mut NetTransport,
    status: i64,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
) -> HttpTaskHandle {
    transport.next_id += 1;
    let id = transport.next_id;
    transport.pending.insert(
        id,
        StreamState::PendingResponse {
            status,
            headers,
            reader: Box::new(io::Cursor::new(body)),
        },
    );
    HttpTaskHandle(id)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Translation helpers
    // -----------------------------------------------------------------------

    #[test]
    fn translate_maps_transport_failures_to_url_error_cases() {
        assert!(matches!(
            translate(ureq::Error::HostNotFound),
            HttpError::Failed { code, .. } if code == "cannotFindHost"
        ));
        assert!(matches!(
            translate(ureq::Error::ConnectionFailed),
            HttpError::Failed { code, .. } if code == "cannotConnectToHost"
        ));
        assert!(matches!(
            translate(ureq::Error::TooManyRedirects),
            HttpError::Failed { code, .. } if code == "httpTooManyRedirects"
        ));
    }

    #[test]
    fn translate_io_error_timed_out_maps_to_timed_out() {
        let e = io::Error::new(io::ErrorKind::TimedOut, "timed out");
        assert_eq!(translate_io_error(&e), "timedOut");
    }

    #[test]
    fn translate_io_error_connection_reset_maps_to_network_connection_lost() {
        let e = io::Error::new(io::ErrorKind::ConnectionReset, "reset");
        assert_eq!(translate_io_error(&e), "networkConnectionLost");
    }

    #[test]
    fn translate_io_error_broken_pipe_maps_to_network_connection_lost() {
        let e = io::Error::new(io::ErrorKind::BrokenPipe, "broken");
        assert_eq!(translate_io_error(&e), "networkConnectionLost");
    }

    // -----------------------------------------------------------------------
    // Chunking logic (in-memory reader — no network)
    // -----------------------------------------------------------------------

    fn collect_events(t: &mut NetTransport, h: HttpTaskHandle) -> Vec<HttpEvent> {
        let mut events = Vec::new();
        loop {
            let e = t.next_event(h);
            let terminal = e.is_terminal();
            events.push(e);
            if terminal {
                break;
            }
        }
        events
    }

    #[test]
    fn empty_body_emits_response_then_done() {
        let mut t = NetTransport::default();
        let h = push_in_memory_stream(&mut t, 200, vec![], vec![]);
        let events = collect_events(&mut t, h);
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            HttpEvent::Response { status: 200, .. }
        ));
        assert_eq!(events[1], HttpEvent::Done);
    }

    #[test]
    fn small_body_emits_response_chunk_done() {
        let mut t = NetTransport::default();
        let h = push_in_memory_stream(&mut t, 200, vec![], b"hello".to_vec());
        let events = collect_events(&mut t, h);
        assert_eq!(events.len(), 3);
        assert!(matches!(
            &events[0],
            HttpEvent::Response { status: 200, .. }
        ));
        assert_eq!(events[1], HttpEvent::Chunk(b"hello".to_vec()));
        assert_eq!(events[2], HttpEvent::Done);
    }

    #[test]
    fn body_larger_than_chunk_size_emits_multiple_chunks() {
        // Body is 2.5 × CHUNK_SIZE — expect 3 Chunk events.
        let body_len = CHUNK_SIZE * 2 + CHUNK_SIZE / 2;
        let body: Vec<u8> = (0..body_len).map(|i| (i % 251) as u8).collect();
        let mut t = NetTransport::default();
        let h = push_in_memory_stream(&mut t, 200, vec![], body.clone());
        let events = collect_events(&mut t, h);

        // Response + 3 chunks + Done
        assert_eq!(events.len(), 5, "events: {events:?}");
        assert!(matches!(
            &events[0],
            HttpEvent::Response { status: 200, .. }
        ));
        assert_eq!(events[4], HttpEvent::Done);

        // Reassemble chunks and verify body round-trips
        let mut assembled = Vec::new();
        for e in &events[1..4] {
            if let HttpEvent::Chunk(bytes) = e {
                assembled.extend_from_slice(bytes);
            }
        }
        assert_eq!(assembled, body);
    }

    #[test]
    fn headers_are_forwarded_in_response_event() {
        let headers = vec![
            ("Content-Type".into(), "application/json".into()),
            ("X-Custom".into(), "value".into()),
        ];
        let mut t = NetTransport::default();
        let h = push_in_memory_stream(&mut t, 201, headers.clone(), vec![]);
        let first = t.next_event(h);
        assert_eq!(
            first,
            HttpEvent::Response {
                status: 201,
                headers
            }
        );
    }

    // -----------------------------------------------------------------------
    // Cancel semantics
    // -----------------------------------------------------------------------

    #[test]
    fn cancel_before_response_event_yields_failed_cancelled() {
        let mut t = NetTransport::default();
        let h = push_in_memory_stream(&mut t, 200, vec![], b"data".to_vec());
        t.cancel(h);
        let e = t.next_event(h);
        assert!(
            matches!(&e, HttpEvent::Failed { code, .. } if code == "cancelled"),
            "expected Failed{{cancelled}}, got {e:?}"
        );
    }

    #[test]
    fn cancel_after_response_event_yields_failed_cancelled() {
        let mut t = NetTransport::default();
        let h = push_in_memory_stream(&mut t, 200, vec![], b"data".to_vec());
        // Consume Response
        let resp = t.next_event(h);
        assert!(matches!(resp, HttpEvent::Response { .. }));
        t.cancel(h);
        let e = t.next_event(h);
        assert!(
            matches!(&e, HttpEvent::Failed { code, .. } if code == "cancelled"),
            "expected Failed{{cancelled}} after mid-stream cancel, got {e:?}"
        );
    }

    #[test]
    fn cancel_yields_cancelled_then_sentinel_on_second_poll() {
        let mut t = NetTransport::default();
        let h = push_in_memory_stream(&mut t, 200, vec![], b"data".to_vec());
        t.cancel(h);
        let terminal = t.next_event(h);
        assert!(
            matches!(&terminal, HttpEvent::Failed { code, .. } if code == "cancelled"),
            "first post-cancel poll must be Failed{{cancelled}}, got {terminal:?}"
        );
        // Handle is now dead — subsequent polls return the sentinel
        let sentinel = t.next_event(h);
        assert!(
            matches!(&sentinel, HttpEvent::Failed { code, .. } if code == "badServerResponse"),
            "expected dead-handle sentinel, got {sentinel:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Dead-handle sentinel
    // -----------------------------------------------------------------------

    #[test]
    fn next_event_on_unknown_handle_returns_sentinel() {
        let mut t = NetTransport::default();
        let unknown = HttpTaskHandle(9999);
        let e = t.next_event(unknown);
        assert!(
            matches!(&e, HttpEvent::Failed { code, .. } if code == "badServerResponse"),
            "expected sentinel for unknown handle, got {e:?}"
        );
    }

    #[test]
    fn next_event_after_done_returns_sentinel() {
        let mut t = NetTransport::default();
        let h = push_in_memory_stream(&mut t, 200, vec![], vec![]);
        // Drain to terminal
        let _ = collect_events(&mut t, h);
        // Any further poll returns the dead-handle sentinel
        let sentinel = t.next_event(h);
        assert!(
            matches!(&sentinel, HttpEvent::Failed { code, .. } if code == "badServerResponse"),
            "expected sentinel after terminal, got {sentinel:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Multiple concurrent handles are independent
    // -----------------------------------------------------------------------

    #[test]
    fn two_concurrent_handles_do_not_interfere() {
        let mut t = NetTransport::default();
        let ha = push_in_memory_stream(&mut t, 200, vec![], b"aaa".to_vec());
        let hb = push_in_memory_stream(&mut t, 201, vec![], b"bbb".to_vec());

        let ea0 = t.next_event(ha); // Response(200) for a
        let eb0 = t.next_event(hb); // Response(201) for b
        let ea1 = t.next_event(ha); // Chunk(aaa)
        let eb1 = t.next_event(hb); // Chunk(bbb)
        let ea2 = t.next_event(ha); // Done
        let eb2 = t.next_event(hb); // Done

        assert!(matches!(ea0, HttpEvent::Response { status: 200, .. }));
        assert!(matches!(eb0, HttpEvent::Response { status: 201, .. }));
        assert_eq!(ea1, HttpEvent::Chunk(b"aaa".to_vec()));
        assert_eq!(eb1, HttpEvent::Chunk(b"bbb".to_vec()));
        assert_eq!(ea2, HttpEvent::Done);
        assert_eq!(eb2, HttpEvent::Done);
    }

    // -----------------------------------------------------------------------
    // Network smoke test (requires --allow-network; NOT run in CI)
    // -----------------------------------------------------------------------

    #[test]
    fn perform_refuses_a_connection_on_an_unroutable_port() {
        // Port 1 on loopback is essentially never listening; the transport
        // must fail with a URLError-shaped code, not panic or hang.
        let mut t = NetTransport::default();
        let err = t
            .perform(&HttpRequest {
                url: "http://127.0.0.1:1/".into(),
                method: "GET".into(),
                headers: Vec::new(),
                body: None,
                timeout_seconds: 2.0,
            })
            .unwrap_err();
        assert!(matches!(err, HttpError::Failed { .. }), "got {err:?}");
    }
}
