//! `URLSession` / `URLSessionConfiguration` / `URLSessionDataTask` on the
//! [`tswift_core::http`] transport seam.
//!
//! ## Architecture (M3 — event-loop driver)
//!
//! The interpreter's executor is cooperative and single-threaded (ADR-0005).
//! All HTTP entry points go through [`run_event_driver`], which drives the
//! `start → next_event* → Done/Failed` event loop defined in ADR-0011.
//! Between events the driver polls [`StdContext::current_task_cancelled`] so
//! a containing `Task.cancel()` triggers `URLError(.cancelled)`.
//!
//! `URLSessionDataTask` is modelled as a `SwiftValue::Struct` whose mutable
//! state (`state`, counters, `progress`, `_cancelled`) lives in struct fields.
//! The `cancel()` / `resume()` intrinsics write the updated receiver back via
//! the normal `Outcome` mechanism — no interior-mutable class machinery needed.
//!
//! Transport failures surface as thrown `URLError` values; a missing transport
//! (sandboxed embedding) is an interpreter error so scripts cannot confuse
//! "no network capability" with a network failure.
//!
//! ## ⚠ VALUE-SEMANTICS LIMITATION — `URLSessionDataTask`
//!
//! **TL;DR:** bind tasks to `var`, never `let`; do not alias through closures
//! and expect the alias to observe `resume()`/`cancel()` mutations.
//!
//! `URLSessionDataTask` is backed by `SwiftValue::Struct` (`Rc<StructObj>`).
//! Mutations made by `cancel()` / `resume()` are written back to the
//! **bound variable** through the `Outcome::receiver` mechanism — exactly as
//! for any other Swift struct.  This diverges from the Swift stdlib where
//! `URLSessionDataTask` is a **reference type** (class), meaning every alias
//! sees the same mutable state:
//!
//! ```swift
//! var task = session.dataTask(with: url) { ... }
//! let snapshot = task   // copies the struct — snapshot.state stays .suspended
//! task.resume()         // writes .running back to `task` only
//! // ⚠ snapshot.state is still .suspended — alias did NOT observe the change
//! ```
//!
//! Fixing this requires backing the task through the class-instance /
//! handle-registry machinery (the same pattern used by SwiftUI session
//! objects).  That work is deferred and tracked as a known limitation in
//! ADR-0011 §Known limitations.  Until then:
//! - Scripts **must** bind tasks to `var`.
//! - Scripts **must not** pass a task into a closure and call `resume()` on the
//!   outer binding expecting the closure to observe the new state.
//! - The `state` field read via `task.state` is always accurate for the
//!   binding that last received the `Outcome::receiver` write-back.

use std::rc::Rc;

use tswift_core::{
    Arg, BuiltinReceiver, EvalError, HttpError, HttpEvent, HttpRequest, LabeledMethodEntry,
    MethodEntry, Outcome, StdContext, StdError, StructObj, SwiftValue,
};

use crate::network::{http_url_response_value, url_error_value};
use crate::type_error;
use crate::url::url_string;
use crate::{data_bytes, data_value};

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register `URLSession`, `URLSessionConfiguration`, and `URLSessionDataTask`
/// on `interp`.
pub(crate) fn install(interp: &mut tswift_core::Interpreter<'_>) {
    // ---- URLSessionTask.State enum ----
    interp.register_builtin_enum(
        "URLSessionTask.State",
        &["running", "suspended", "canceling", "completed"],
    );

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
    interp.register_labeled_intrinsic(
        BuiltinReceiver::URLSession,
        "dataTask",
        LabeledMethodEntry {
            mutating: false,
            func: session_data_task,
        },
    );

    // ---- URLSessionDataTask ----
    interp.register_intrinsic(
        BuiltinReceiver::URLSessionDataTask,
        "cancel",
        MethodEntry {
            mutating: true,
            func: task_cancel,
        },
    );
    interp.register_intrinsic(
        BuiltinReceiver::URLSessionDataTask,
        "resume",
        MethodEntry {
            mutating: true,
            func: task_resume,
        },
    );
}

// ---------------------------------------------------------------------------
// URLSessionConfiguration
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// URLSession
// ---------------------------------------------------------------------------

fn session_value(configuration: SwiftValue) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "URLSession".into(),
        fields: vec![("configuration".into(), configuration)],
    }))
}

fn session_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> tswift_core::StdResult {
    // Accept `URLSession(configuration:)` and
    // `URLSession(configuration:delegate:delegateQueue:)` (delegate/queue ignored).
    let config_arg = args
        .iter()
        .find(|a| a.label.as_deref() == Some("configuration"));
    match config_arg.map(|a| &a.value) {
        Some(SwiftValue::Struct(o)) if o.type_name == "URLSessionConfiguration" => {
            Ok(session_value(config_arg.unwrap().value.clone()))
        }
        Some(_) => Err(type_error(
            "URLSession(configuration:) expects a URLSessionConfiguration",
        )),
        None => Err(type_error("URLSession(configuration:) expects one label")),
    }
}

fn session_configuration(recv: SwiftValue) -> tswift_core::StdResult {
    match &recv {
        SwiftValue::Struct(o) if o.type_name == "URLSession" => Ok(o
            .get("configuration")
            .cloned()
            .unwrap_or_else(configuration_value)),
        _ => Err(type_error("configuration expects URLSession")),
    }
}

/// Extract the session's request timeout from its configuration.
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

// ---------------------------------------------------------------------------
// URLRequest lowering
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Event-loop driver (M3 seam)
// ---------------------------------------------------------------------------

/// Outcome of a successful [`run_event_driver`] call.
#[derive(Debug)]
struct DriverOutcome {
    body: Vec<u8>,
    status: i64,
    headers: Vec<(String, String)>,
    /// URL that was requested.
    url: String,
    /// Bytes received (sum of all Chunk payloads).
    bytes_received: i64,
    /// Expected byte count from `Content-Length`, or -1 if unknown.
    bytes_expected: i64,
}

/// Maximum non-terminal events to drain after cancel or a malformed event
/// sequence.  Prevents a misbehaving transport from spinning forever.
const MAX_DRAIN_EVENTS: usize = 1_000;

/// Drive the request event loop: `start → next_event* → Done/Failed`.
///
/// Checks [`StdContext::current_task_cancelled`] before starting and after
/// each event, so a containing `Task.cancel()` surfaces as
/// `URLError(.cancelled)` without touching the transport. Event-order
/// violations (no `Response` before terminal, malformed sequence) map to
/// `badServerResponse`.
///
/// Callers must NOT call [`StdContext::perform_http`]; they must use this
/// driver directly so delegate hooks and cancellation checks compose
/// correctly (M3 seam contract from notes.md).
fn run_event_driver(
    ctx: &mut dyn StdContext,
    req: &HttpRequest,
) -> Result<DriverOutcome, StdError> {
    // Honour cooperative cancellation before touching the transport.
    if ctx.current_task_cancelled() {
        return Err(StdError::Throw(url_error_value(
            "cancelled",
            crate::url::url_value(req.url.clone()),
        )));
    }

    let handle = match ctx.http_start(req) {
        Ok(h) => h,
        Err(HttpError::Unavailable) => {
            return Err(StdError::Error(EvalError::Unsupported(
                "URLSession needs a network transport; this embedding has none configured".into(),
            )));
        }
        Err(HttpError::Failed { code, message: _ }) => {
            return Err(StdError::Throw(url_error_value(
                &code,
                crate::url::url_value(req.url.clone()),
            )));
        }
    };

    let url = req.url.clone();
    let mut body: Vec<u8> = Vec::new();
    let mut status: i64 = 0;
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut got_response = false;
    let mut bytes_expected: i64 = -1;

    loop {
        let event = ctx.http_next_event(handle);
        match event {
            HttpEvent::Response {
                status: s,
                headers: h,
            } => {
                if got_response {
                    // Second Response in the same stream — malformed sequence
                    // per ADR-0011.  Cancel transport, drain to terminal, then
                    // surface badServerResponse.
                    ctx.http_cancel(handle);
                    let mut drain = 0usize;
                    loop {
                        if ctx.http_next_event(handle).is_terminal() {
                            break;
                        }
                        drain += 1;
                        if drain >= MAX_DRAIN_EVENTS {
                            break;
                        }
                    }
                    return Err(StdError::Throw(url_error_value(
                        "badServerResponse",
                        crate::url::url_value(url),
                    )));
                }
                status = s;
                // Extract Content-Length for progress tracking.
                for (name, val) in &h {
                    if name.eq_ignore_ascii_case("content-length") {
                        if let Ok(n) = val.parse::<i64>() {
                            bytes_expected = n;
                        }
                    }
                }
                headers = h;
                got_response = true;
            }
            HttpEvent::Chunk(bytes) => {
                if !got_response {
                    // Chunk before Response — malformed sequence per ADR-0011.
                    // Cancel transport, drain to terminal, then surface
                    // badServerResponse.
                    ctx.http_cancel(handle);
                    let mut drain = 0usize;
                    loop {
                        if ctx.http_next_event(handle).is_terminal() {
                            break;
                        }
                        drain += 1;
                        if drain >= MAX_DRAIN_EVENTS {
                            break;
                        }
                    }
                    return Err(StdError::Throw(url_error_value(
                        "badServerResponse",
                        crate::url::url_value(url),
                    )));
                }
                body.extend_from_slice(&bytes);
            }
            HttpEvent::Done => {
                if !got_response {
                    // Terminal without a Response event — malformed sequence.
                    return Err(StdError::Throw(url_error_value(
                        "badServerResponse",
                        crate::url::url_value(url),
                    )));
                }
                break;
            }
            HttpEvent::Failed { code, message: _ } => {
                // Failed events are transport errors (e.g., connection refused,
                // DNS failure, or mid-flight network drop).  A Failed event
                // *before* Response is a legitimate transport failure, not a
                // protocol violation — surface it as URLError with the
                // transport's code rather than badServerResponse.
                return Err(StdError::Throw(url_error_value(
                    &code,
                    crate::url::url_value(url),
                )));
            }
        }

        // Poll cooperative cancellation between events. Cancel the transport
        // and drain it to terminal before returning (drain-or-cancel invariant
        // from notes.md: every started handle MUST be drained to terminal or
        // cancelled+drained).
        // Poll cooperative cancellation between events.  Cancel the transport
        // and drain to terminal before returning (drain-or-cancel invariant:
        // every started handle MUST be drained to terminal or
        // cancelled+drained).  The drain loop is capped at MAX_DRAIN_EVENTS to
        // guard against a misbehaving transport.
        if ctx.current_task_cancelled() {
            ctx.http_cancel(handle);
            let mut drain = 0usize;
            loop {
                if ctx.http_next_event(handle).is_terminal() {
                    break;
                }
                drain += 1;
                if drain >= MAX_DRAIN_EVENTS {
                    break;
                }
            }
            return Err(StdError::Throw(url_error_value(
                "cancelled",
                crate::url::url_value(url),
            )));
        }
    }

    let bytes_received = body.len() as i64;
    Ok(DriverOutcome {
        body,
        status,
        headers,
        url,
        bytes_received,
        bytes_expected,
    })
}

/// Build the `(Data, URLResponse)` tuple from a [`DriverOutcome`].
fn driver_outcome_to_tuple(outcome: DriverOutcome) -> SwiftValue {
    let response = http_url_response_value(
        crate::url::url_value(outcome.url),
        i128::from(outcome.status),
        outcome.headers,
    );
    SwiftValue::Tuple(vec![data_value(outcome.body), response], vec![None, None])
}

// ---------------------------------------------------------------------------
// Session async methods (data/upload) — rerouted through the driver
// ---------------------------------------------------------------------------

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
    let outcome = run_event_driver(ctx, &req)?;
    let result = driver_outcome_to_tuple(outcome);
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
    let outcome = run_event_driver(ctx, &req)?;
    let result = driver_outcome_to_tuple(outcome);
    Ok(Some(Outcome {
        result,
        receiver: recv,
    }))
}

// ---------------------------------------------------------------------------
// URLSessionDataTask
// ---------------------------------------------------------------------------

/// Build the `URLSessionTask.State` enum value for case `case_name`.
fn task_state_value(case_name: &str) -> SwiftValue {
    SwiftValue::Enum(Rc::new(tswift_core::EnumObj {
        type_name: "URLSessionTask.State".into(),
        case: case_name.into(),
        payload: Vec::new(),
    }))
}

/// Build a `Progress` struct with the given `fractionCompleted`.
fn progress_value(fraction: f64) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "Progress".into(),
        fields: vec![("fractionCompleted".into(), SwiftValue::Double(fraction))],
    }))
}

/// Build a fresh suspended `URLSessionDataTask` struct.
///
/// `req_value` is a `URLRequest` struct (or URL to be lowered at resume time).
/// `closure_id` is the index into the interpreter's closure table for the
/// completion handler; `-1` means no handler.
fn task_value(req_value: SwiftValue, session_timeout: f64, closure_id: i128) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "URLSessionDataTask".into(),
        fields: vec![
            // Private: request, timeout, closure
            ("_req".into(), req_value),
            (
                "_session_timeout".into(),
                SwiftValue::Double(session_timeout),
            ),
            ("_closure_id".into(), SwiftValue::int(closure_id)),
            // Private: cancelled flag
            ("_cancelled".into(), SwiftValue::Bool(false)),
            // Public state
            ("state".into(), task_state_value("suspended")),
            ("countOfBytesReceived".into(), SwiftValue::int(0)),
            ("countOfBytesExpectedToReceive".into(), SwiftValue::int(-1)),
            ("progress".into(), progress_value(0.0)),
        ],
    }))
}

/// `URLSession.dataTask(with: URL|URLRequest, completionHandler: ...)`.
///
/// The trailing closure (completion handler) is captured by closure ID.
/// Calling `task.resume()` drives the event loop and then invokes the handler
/// `(Data?, URLResponse?, Error?) -> Void` inline (cooperative executor).
fn session_data_task(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> Result<Option<Outcome>, StdError> {
    // Expect: label "with" + URL/URLRequest, then an unlabeled/trailing closure.
    let with_arg = args.iter().find(|a| a.label.as_deref() == Some("with"));
    let closure_arg = args.iter().find(|a| {
        // trailing closure: unlabeled or labeled "completionHandler"
        a.label.is_none() || a.label.as_deref() == Some("completionHandler")
    });

    let with_arg = match with_arg {
        Some(a) => a,
        None => return Ok(None),
    };

    // Normalise: if with_arg is a URL, wrap it in a URLRequest struct.
    let req_value = match &with_arg.value {
        SwiftValue::Struct(o) if o.type_name == "URLRequest" => with_arg.value.clone(),
        url_val => {
            // Build a minimal URLRequest from the URL value.
            let url_str = url_string(url_val)?;
            let timeout = session_timeout(&recv);
            crate::network::url_request_value(
                crate::url::url_value(url_str),
                timeout,
                SwiftValue::Nil,
                SwiftValue::Str("GET".into()),
                SwiftValue::Nil,
            )
        }
    };

    let closure_id = match closure_arg.map(|a| &a.value) {
        Some(SwiftValue::Closure(id)) => *id as i128,
        _ => -1,
    };

    let timeout = session_timeout(&recv);
    let task = task_value(req_value, timeout, closure_id);
    Ok(Some(Outcome {
        result: task,
        receiver: recv,
    }))
}

/// `URLSessionDataTask.cancel()` — mark the task as canceling.
///
/// If `resume()` has not been called yet, the next `resume()` will complete
/// with `URLError(.cancelled)` without touching the transport.
fn task_cancel(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let SwiftValue::Struct(ref task) = recv else {
        return Ok(Outcome {
            result: SwiftValue::Void,
            receiver: recv,
        });
    };
    // Idempotent: already canceling or completed — no-op.
    let already_done = matches!(
        task.get("state"),
        Some(SwiftValue::Enum(e)) if e.case == "canceling" || e.case == "completed"
    );
    if already_done {
        return Ok(Outcome {
            result: SwiftValue::Void,
            receiver: recv,
        });
    }
    let mut updated: StructObj = (**task).clone();
    updated.set("_cancelled", SwiftValue::Bool(true));
    updated.set("state", task_state_value("canceling"));
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: SwiftValue::Struct(Rc::new(updated)),
    })
}

/// `URLSessionDataTask.resume()` — drive the event loop then invoke the
/// completion handler.
///
/// State transitions: `suspended` → `running` → `completed` (or `canceling`
/// if cancelled).
fn task_resume(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let SwiftValue::Struct(ref task) = recv else {
        return Ok(Outcome {
            result: SwiftValue::Void,
            receiver: recv,
        });
    };

    // State guard: Foundation semantics — resume() on a running or completed
    // task is a no-op.  A .canceling task is allowed to proceed so that the
    // pre-flight cancel path (pre_cancelled flag below) can deliver URLError
    // to the completion handler, matching Foundation behaviour.
    let should_proceed = matches!(
        task.get("state"),
        Some(SwiftValue::Enum(e)) if e.case == "suspended" || e.case == "canceling"
    );
    if !should_proceed {
        return Ok(Outcome {
            result: SwiftValue::Void,
            receiver: recv,
        });
    }

    // Extract closure id (-1 → no handler).
    let closure_id = match task.get("_closure_id") {
        Some(SwiftValue::Int(i)) => i.raw,
        _ => -1,
    };

    // Extract the stored URLRequest.
    let req_value = task.get("_req").cloned().unwrap_or(SwiftValue::Nil);
    let session_timeout = match task.get("_session_timeout") {
        Some(SwiftValue::Double(d)) => *d,
        _ => 60.0,
    };
    // Ensure the URLRequest has its timeoutInterval set to the session timeout
    // when not explicitly overridden.
    let mut req = lower_request(&req_value)?;
    if req.timeout_seconds == 60.0 {
        req.timeout_seconds = session_timeout;
    }

    // Pre-flight cancel: deliver URLError(.cancelled) to the handler without
    // touching the transport.
    let pre_cancelled = matches!(task.get("_cancelled"), Some(SwiftValue::Bool(true)));
    if pre_cancelled {
        let mut updated: StructObj = (**task).clone();
        updated.set("state", task_state_value("completed"));
        let updated_recv = SwiftValue::Struct(Rc::new(updated));
        if closure_id >= 0 {
            let err = url_error_value("cancelled", crate::url::url_value(req.url));
            ctx.call_closure(
                closure_id as usize,
                vec![SwiftValue::Nil, SwiftValue::Nil, err],
            )?;
        }
        return Ok(Outcome {
            result: SwiftValue::Void,
            receiver: updated_recv,
        });
    }

    // Mark running.
    let mut updated: StructObj = (**task).clone();
    updated.set("state", task_state_value("running"));

    // Drive the event loop.
    match run_event_driver(ctx, &req) {
        Ok(outcome) => {
            let bytes_received = outcome.bytes_received;
            let bytes_expected = outcome.bytes_expected;
            let fraction = if bytes_expected > 0 {
                bytes_received as f64 / bytes_expected as f64
            } else {
                1.0 // unknown length: report 1.0 on success
            };

            let response = http_url_response_value(
                crate::url::url_value(outcome.url),
                i128::from(outcome.status),
                outcome.headers,
            );
            let data = data_value(outcome.body);

            // Update counters and progress on the task struct.
            updated.set(
                "countOfBytesReceived",
                SwiftValue::int(bytes_received as i128),
            );
            updated.set(
                "countOfBytesExpectedToReceive",
                SwiftValue::int(if bytes_expected >= 0 {
                    bytes_expected as i128
                } else {
                    -1
                }),
            );
            updated.set("progress", progress_value(fraction));
            updated.set("state", task_state_value("completed"));
            let updated_recv = SwiftValue::Struct(Rc::new(updated));

            // Invoke completion handler: (Data, URLResponse, nil).
            if closure_id >= 0 {
                ctx.call_closure(closure_id as usize, vec![data, response, SwiftValue::Nil])?;
            }

            Ok(Outcome {
                result: SwiftValue::Void,
                receiver: updated_recv,
            })
        }
        Err(err) => {
            // Transport error: mark task completed, deliver error to handler.
            updated.set("state", task_state_value("completed"));
            let updated_recv = SwiftValue::Struct(Rc::new(updated));

            let error_value = match &err {
                StdError::Throw(v) => v.clone(),
                _ => url_error_value("unknown", crate::url::url_value(req.url.clone())),
            };

            if closure_id >= 0 {
                ctx.call_closure(
                    closure_id as usize,
                    vec![SwiftValue::Nil, SwiftValue::Nil, error_value],
                )?;
            }

            // For `task.resume()` the completion handler absorbed the error;
            // `resume()` itself returns normally (the error is delivered to
            // the handler, not re-thrown to the call site).
            Ok(Outcome {
                result: SwiftValue::Void,
                receiver: updated_recv,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::url::url_value;
    use tswift_core::{
        HttpEvent, HttpRequest, HttpResponse, HttpTaskHandle, MockChunkedRoute, MockHttpTransport,
        MockRoute,
    };

    /// A minimal `StdContext` that uses the event-loop seam of a
    /// `MockHttpTransport`, so tests exercise the M3 driver correctly.
    struct HttpEventCtx {
        transport: MockHttpTransport,
        out: Vec<u8>,
        calls: Vec<(usize, Vec<SwiftValue>)>,
        /// Counts how many times `http_start` was called (used to verify
        /// that double-resume does not issue a second request).
        start_count: usize,
    }

    impl HttpEventCtx {
        fn new(routes: Vec<MockRoute>) -> Self {
            Self {
                transport: MockHttpTransport::new(routes),
                out: Vec::new(),
                calls: Vec::new(),
                start_count: 0,
            }
        }

        fn with_chunked(routes: Vec<MockChunkedRoute>) -> Self {
            Self {
                transport: MockHttpTransport::default().with_chunked_routes(routes),
                out: Vec::new(),
                calls: Vec::new(),
                start_count: 0,
            }
        }
    }

    impl StdContext for HttpEventCtx {
        fn call_closure(&mut self, id: usize, args: Vec<SwiftValue>) -> tswift_core::StdResult {
            self.calls.push((id, args));
            Ok(SwiftValue::Void)
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            &mut self.out
        }
        fn http_start(
            &mut self,
            req: &HttpRequest,
        ) -> Result<HttpTaskHandle, tswift_core::HttpError> {
            use tswift_core::HttpTransport;
            self.start_count += 1;
            self.transport.start(req)
        }
        fn http_next_event(&mut self, h: HttpTaskHandle) -> HttpEvent {
            use tswift_core::HttpTransport;
            self.transport.next_event(h)
        }
        fn http_cancel(&mut self, h: HttpTaskHandle) {
            use tswift_core::HttpTransport;
            self.transport.cancel(h)
        }
    }

    /// A `StdContext` that replays a fixed sequence of `HttpEvent`s, used to
    /// inject malformed or cancelled event streams in unit tests.
    struct SequenceCtx {
        events: std::collections::VecDeque<HttpEvent>,
        out: Vec<u8>,
        http_cancel_called: bool,
        /// Becomes true for `current_task_cancelled()` after this many events
        /// have been popped by `http_next_event`. `usize::MAX` means never.
        cancel_after_events: usize,
        events_consumed: std::cell::Cell<usize>,
    }

    impl SequenceCtx {
        fn new(events: Vec<HttpEvent>) -> Self {
            Self {
                events: events.into(),
                out: Vec::new(),
                http_cancel_called: false,
                cancel_after_events: usize::MAX,
                events_consumed: std::cell::Cell::new(0),
            }
        }

        fn cancel_after(mut self, n: usize) -> Self {
            self.cancel_after_events = n;
            self
        }
    }

    impl StdContext for SequenceCtx {
        fn call_closure(&mut self, _id: usize, _args: Vec<SwiftValue>) -> tswift_core::StdResult {
            Ok(SwiftValue::Void)
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            &mut self.out
        }
        fn http_start(
            &mut self,
            _req: &HttpRequest,
        ) -> Result<HttpTaskHandle, tswift_core::HttpError> {
            Ok(HttpTaskHandle(1))
        }
        fn http_next_event(&mut self, _h: HttpTaskHandle) -> HttpEvent {
            let ev = self.events.pop_front().unwrap_or(HttpEvent::Failed {
                code: "badServerResponse".into(),
                message: "no more events".into(),
            });
            self.events_consumed.set(self.events_consumed.get() + 1);
            ev
        }
        fn http_cancel(&mut self, _h: HttpTaskHandle) {
            self.http_cancel_called = true;
            // Append a terminal Failed{cancelled} at the end of the queue so the
            // drain loop in run_event_driver will eventually hit a terminal event.
            // Non-terminal events already in the queue remain so the drain loop
            // exercises the "consume until terminal" path.
            self.events.push_back(HttpEvent::Failed {
                code: "cancelled".into(),
                message: String::new(),
            });
        }
        fn current_task_cancelled(&self) -> bool {
            self.events_consumed.get() >= self.cancel_after_events
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

    fn ok_resp(status: i64, body: &[u8]) -> HttpResponse {
        HttpResponse {
            status,
            headers: vec![("Content-Length".into(), body.len().to_string())],
            body: body.to_vec(),
        }
    }

    // -----------------------------------------------------------------------
    // lower_request
    // -----------------------------------------------------------------------

    #[test]
    fn lower_request_carries_url_method_and_timeout() {
        let req = lower_request(&request_value("https://example.com/a")).unwrap();
        assert_eq!(req.url, "https://example.com/a");
        assert_eq!(req.method, "GET");
        assert_eq!(req.timeout_seconds, 60.0);
        assert_eq!(req.body, None);
        assert!(req.headers.is_empty());
    }

    // -----------------------------------------------------------------------
    // run_event_driver — basic cases
    // -----------------------------------------------------------------------

    #[test]
    fn driver_returns_data_and_response_on_success() {
        let mut ctx = HttpEventCtx::new(vec![MockRoute {
            method: "GET".into(),
            url: "https://example.com/hello".into(),
            outcome: Ok(ok_resp(200, b"hello")),
        }]);
        let req = HttpRequest {
            url: "https://example.com/hello".into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: None,
            timeout_seconds: 60.0,
        };
        let out = run_event_driver(&mut ctx, &req).unwrap();
        assert_eq!(out.body, b"hello");
        assert_eq!(out.status, 200);
        assert_eq!(out.bytes_received, 5);
        assert_eq!(out.bytes_expected, 5);
    }

    #[test]
    fn driver_throws_url_error_on_transport_failure() {
        let mut ctx = HttpEventCtx::new(vec![]);
        let req = HttpRequest {
            url: "https://nowhere.invalid/".into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: None,
            timeout_seconds: 60.0,
        };
        let err = run_event_driver(&mut ctx, &req).unwrap_err();
        let StdError::Throw(SwiftValue::Struct(o)) = err else {
            panic!("expected thrown URLError, got {err:?}");
        };
        assert_eq!(o.type_name, "URLError");
    }

    #[test]
    fn driver_throws_url_error_on_cancelled_context() {
        struct CancelledCtx(Vec<u8>);
        impl StdContext for CancelledCtx {
            fn call_closure(
                &mut self,
                _id: usize,
                _args: Vec<SwiftValue>,
            ) -> tswift_core::StdResult {
                Err(type_error("unused"))
            }
            fn out(&mut self) -> &mut dyn std::io::Write {
                &mut self.0
            }
            fn current_task_cancelled(&self) -> bool {
                true
            }
        }
        let mut ctx = CancelledCtx(Vec::new());
        let req = HttpRequest {
            url: "https://example.com/".into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: None,
            timeout_seconds: 60.0,
        };
        let err = run_event_driver(&mut ctx, &req).unwrap_err();
        let StdError::Throw(SwiftValue::Struct(o)) = err else {
            panic!("expected thrown URLError, got {err:?}");
        };
        assert_eq!(o.type_name, "URLError");
        let url_code = o.get("code");
        if let Some(SwiftValue::Enum(e)) = url_code {
            assert_eq!(e.case, "cancelled");
        }
    }

    #[test]
    fn driver_missing_transport_is_interpreter_error() {
        struct NoNet(Vec<u8>);
        impl StdContext for NoNet {
            fn call_closure(
                &mut self,
                _id: usize,
                _args: Vec<SwiftValue>,
            ) -> tswift_core::StdResult {
                Err(type_error("unused"))
            }
            fn out(&mut self) -> &mut dyn std::io::Write {
                &mut self.0
            }
            // http_start returns Unavailable by default
        }
        let mut ctx = NoNet(Vec::new());
        let req = HttpRequest {
            url: "https://example.com/".into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: None,
            timeout_seconds: 60.0,
        };
        let err = run_event_driver(&mut ctx, &req).unwrap_err();
        assert!(matches!(err, StdError::Error(EvalError::Unsupported(_))));
    }

    // -----------------------------------------------------------------------
    // session_data / session_upload (async paths using event driver)
    // -----------------------------------------------------------------------

    #[test]
    fn data_from_url_returns_data_and_response_tuple() {
        let mut ctx = HttpEventCtx::new(vec![MockRoute {
            method: "GET".into(),
            url: "https://example.com/hello".into(),
            outcome: Ok(ok_resp(200, b"hello")),
        }]);
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
        let mut ctx = HttpEventCtx::new(vec![]);
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
    fn upload_overrides_the_request_body() {
        let mut ctx = HttpEventCtx::new(vec![MockRoute {
            method: "GET".into(),
            url: "https://example.com/up".into(),
            outcome: Ok(HttpResponse {
                status: 201,
                headers: Vec::new(),
                body: Vec::new(),
            }),
        }]);
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

    // -----------------------------------------------------------------------
    // URLSessionDataTask — cancel before resume
    // -----------------------------------------------------------------------

    #[test]
    fn task_cancel_sets_cancelled_flag_and_state() {
        let task = task_value(request_value("https://example.com/"), 60.0, 0);
        let mut ctx = HttpEventCtx::new(vec![]);
        let outcome = task_cancel(&mut ctx, task, vec![]).unwrap();
        let SwiftValue::Struct(updated) = &outcome.receiver else {
            panic!("expected struct receiver");
        };
        assert_eq!(updated.get("_cancelled"), Some(&SwiftValue::Bool(true)));
        let state = updated.get("state");
        assert!(
            matches!(state, Some(SwiftValue::Enum(e)) if e.case == "canceling"),
            "expected state=canceling, got {state:?}"
        );
    }

    #[test]
    fn task_resume_after_cancel_calls_handler_with_url_error() {
        let task = task_value(request_value("https://example.com/"), 60.0, 0);
        // Cancel first.
        let mut ctx = HttpEventCtx::new(vec![]);
        let cancel_out = task_cancel(&mut ctx, task, vec![]).unwrap();
        let cancelled_task = cancel_out.receiver;

        // Resume — should call handler with URLError, not touch transport.
        let outcome = task_resume(&mut ctx, cancelled_task, vec![]).unwrap();
        // Exactly one closure call, third arg is the URLError.
        assert_eq!(ctx.calls.len(), 1, "expected one handler call");
        let args = &ctx.calls[0].1;
        assert_eq!(args[0], SwiftValue::Nil, "data should be nil");
        assert_eq!(args[1], SwiftValue::Nil, "response should be nil");
        let SwiftValue::Struct(err_struct) = &args[2] else {
            panic!("expected URLError struct as third arg, got {:?}", args[2]);
        };
        assert_eq!(err_struct.type_name, "URLError");
        // State should be completed.
        let SwiftValue::Struct(task_struct) = &outcome.receiver else {
            panic!("expected struct receiver after resume");
        };
        let state = task_struct.get("state");
        assert!(
            matches!(state, Some(SwiftValue::Enum(e)) if e.case == "completed"),
            "expected state=completed, got {state:?}"
        );
    }

    // -----------------------------------------------------------------------
    // URLSessionDataTask — happy path resume
    // -----------------------------------------------------------------------

    #[test]
    fn task_resume_happy_path_calls_handler_with_data_and_response() {
        let mut ctx = HttpEventCtx::new(vec![MockRoute {
            method: "GET".into(),
            url: "https://example.com/hello".into(),
            outcome: Ok(ok_resp(200, b"hello")),
        }]);
        let task = task_value(request_value("https://example.com/hello"), 60.0, 42);
        let outcome = task_resume(&mut ctx, task, vec![]).unwrap();

        // Handler called with (data, response, nil).
        assert_eq!(ctx.calls.len(), 1);
        let args = &ctx.calls[0].1;
        assert_eq!(ctx.calls[0].0, 42, "closure id");
        assert_eq!(data_bytes(&args[0]).unwrap(), b"hello".to_vec());
        let SwiftValue::Struct(resp) = &args[1] else {
            panic!("expected HTTPURLResponse");
        };
        assert_eq!(resp.type_name, "HTTPURLResponse");
        assert_eq!(args[2], SwiftValue::Nil, "error should be nil");

        // Task state updated.
        let SwiftValue::Struct(t) = &outcome.receiver else {
            panic!("expected struct");
        };
        assert_eq!(t.get("countOfBytesReceived"), Some(&SwiftValue::int(5)));
        assert_eq!(
            t.get("countOfBytesExpectedToReceive"),
            Some(&SwiftValue::int(5))
        );
        let state = t.get("state");
        assert!(
            matches!(state, Some(SwiftValue::Enum(e)) if e.case == "completed"),
            "expected completed, got {state:?}"
        );
    }

    // -----------------------------------------------------------------------
    // run_event_driver — malformed event-order sequences (ADR-0011)
    // -----------------------------------------------------------------------

    #[test]
    fn chunk_before_response_is_bad_server_response() {
        // Chunk arrives before the Response event — malformed per ADR-0011.
        let mut ctx = SequenceCtx::new(vec![
            HttpEvent::Chunk(b"early data".to_vec()),
            HttpEvent::Done,
        ]);
        let req = HttpRequest {
            url: "https://example.com/".into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: None,
            timeout_seconds: 60.0,
        };
        let err = run_event_driver(&mut ctx, &req).unwrap_err();
        let StdError::Throw(SwiftValue::Struct(o)) = err else {
            panic!("expected thrown URLError, got {err:?}");
        };
        assert_eq!(o.type_name, "URLError");
        assert!(
            matches!(o.get("code"), Some(SwiftValue::Enum(e)) if e.case == "badServerResponse"),
            "expected badServerResponse, got {:?}",
            o.get("code")
        );
        // http_cancel must have been called so the transport can be cleaned up.
        assert!(
            ctx.http_cancel_called,
            "http_cancel must be called on malformed stream"
        );
    }

    #[test]
    fn double_response_is_bad_server_response() {
        // Two Response events in one stream — malformed per ADR-0011.
        let mut ctx = SequenceCtx::new(vec![
            HttpEvent::Response {
                status: 200,
                headers: vec![],
            },
            HttpEvent::Response {
                status: 200,
                headers: vec![],
            },
            HttpEvent::Done,
        ]);
        let req = HttpRequest {
            url: "https://example.com/".into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: None,
            timeout_seconds: 60.0,
        };
        let err = run_event_driver(&mut ctx, &req).unwrap_err();
        let StdError::Throw(SwiftValue::Struct(o)) = err else {
            panic!("expected thrown URLError, got {err:?}");
        };
        assert_eq!(o.type_name, "URLError");
        assert!(
            matches!(o.get("code"), Some(SwiftValue::Enum(e)) if e.case == "badServerResponse"),
            "expected badServerResponse, got {:?}",
            o.get("code")
        );
        assert!(
            ctx.http_cancel_called,
            "http_cancel must be called on malformed stream"
        );
    }

    // -----------------------------------------------------------------------
    // run_event_driver — mid-flight cooperative cancellation
    // -----------------------------------------------------------------------

    #[test]
    fn mid_flight_cancel_drains_transport_and_returns_url_error() {
        // Simulate a stream that has delivered one Response event before the
        // outer Task is cancelled (current_task_cancelled flips true after
        // events_consumed >= 1).  The driver must:
        //   1. call http_cancel on the transport handle,
        //   2. drain the remaining events to the terminal Failed{cancelled},
        //   3. return URLError(.cancelled).
        // Sequence: Response (consumed in normal loop), then two Chunks that
        // are still in the queue when cancel fires.  http_cancel appends a
        // Failed{cancelled} terminal, giving drain loop: Chunk + Chunk + Failed.
        // cancel_after_events=1 makes current_task_cancelled() true as soon as
        // the Response has been consumed (events_consumed >= 1).
        let mut ctx = SequenceCtx::new(vec![
            HttpEvent::Response {
                status: 200,
                headers: vec![],
            },
            // Chunks buffered in the queue; consumed by the drain loop.
            HttpEvent::Chunk(b"part1".to_vec()),
            HttpEvent::Chunk(b"part2".to_vec()),
            // http_cancel will append Failed{cancelled} — no pre-insert needed.
        ])
        .cancel_after(1); // cancel_after_events = 1 → true once Response is consumed

        let req = HttpRequest {
            url: "https://example.com/stream".into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: None,
            timeout_seconds: 60.0,
        };
        let err = run_event_driver(&mut ctx, &req).unwrap_err();

        // Driver must have called http_cancel.
        assert!(
            ctx.http_cancel_called,
            "http_cancel must be called on mid-flight cancel"
        );

        // The queue must be fully drained (no events left unread).
        assert!(
            ctx.events.is_empty(),
            "drain loop must consume all remaining events; {} left",
            ctx.events.len()
        );

        // URLError(.cancelled) must be thrown.
        let StdError::Throw(SwiftValue::Struct(o)) = err else {
            panic!("expected thrown URLError, got {err:?}");
        };
        assert_eq!(o.type_name, "URLError");
        assert!(
            matches!(o.get("code"), Some(SwiftValue::Enum(e)) if e.case == "cancelled"),
            "expected URLError(.cancelled), got {:?}",
            o.get("code")
        );
    }

    // -----------------------------------------------------------------------
    // URLSessionDataTask — double resume is a no-op
    // -----------------------------------------------------------------------

    #[test]
    fn double_resume_issues_exactly_one_request() {
        let mut ctx = HttpEventCtx::new(vec![MockRoute {
            method: "GET".into(),
            url: "https://example.com/once".into(),
            outcome: Ok(ok_resp(200, b"ok")),
        }]);
        let task = task_value(request_value("https://example.com/once"), 60.0, 99);

        // First resume: suspended → running → completed.
        let out1 = task_resume(&mut ctx, task, vec![]).unwrap();
        assert_eq!(
            ctx.start_count, 1,
            "first resume must issue exactly one request"
        );
        assert_eq!(ctx.calls.len(), 1, "handler called once after first resume");

        // Second resume: state is .completed → state guard returns early, no
        // transport call, no additional handler invocation.
        let completed_task = out1.receiver;
        let _out2 = task_resume(&mut ctx, completed_task, vec![]).unwrap();
        assert_eq!(
            ctx.start_count, 1,
            "second resume must NOT issue another request"
        );
        assert_eq!(
            ctx.calls.len(),
            1,
            "completion handler must NOT be called a second time"
        );
    }

    // -----------------------------------------------------------------------
    // URLSessionDataTask — chunked route (progress counters)
    // -----------------------------------------------------------------------

    #[test]
    fn task_resume_chunked_accumulates_bytes_and_sets_progress() {
        let mut ctx = HttpEventCtx::with_chunked(vec![MockChunkedRoute {
            method: "GET".into(),
            url: "https://stream.example.com/".into(),
            status: 200,
            headers: vec![("Content-Length".into(), "12".into())],
            chunks: vec![b"chunk1".to_vec(), b"chunk2".to_vec()],
            fail_after_chunks: None,
        }]);
        let task = task_value(request_value("https://stream.example.com/"), 60.0, 7);
        let outcome = task_resume(&mut ctx, task, vec![]).unwrap();

        // Handler called once with all data concatenated.
        assert_eq!(ctx.calls.len(), 1);
        let args = &ctx.calls[0].1;
        assert_eq!(data_bytes(&args[0]).unwrap(), b"chunk1chunk2".to_vec());

        let SwiftValue::Struct(t) = &outcome.receiver else {
            panic!("expected struct");
        };
        assert_eq!(t.get("countOfBytesReceived"), Some(&SwiftValue::int(12)));
        assert_eq!(
            t.get("countOfBytesExpectedToReceive"),
            Some(&SwiftValue::int(12))
        );
        let prog = t.get("progress");
        assert!(
            matches!(prog, Some(SwiftValue::Struct(s)) if {
                matches!(s.get("fractionCompleted"), Some(SwiftValue::Double(f)) if (*f - 1.0).abs() < 1e-9)
            }),
            "expected fractionCompleted=1.0, got {prog:?}"
        );
    }
}
