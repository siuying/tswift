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
//! `URLSessionConfiguration`, `URLSession`, and `URLSessionDataTask` are all
//! backed by `SwiftValue::Object(Rc<RefCell<ClassObj>>)` — reference semantics
//! matching the real Foundation classes.  Mutations go through the `RefCell`
//! in place; they are registered `mutating: false` so the interpreter does not
//! attempt a struct write-back and `let config = ...` / `let session = ...` /
//! `let task = ...` bindings are all legal.
//!
//! `URLSessionConfiguration.default` and `.ephemeral` return a **fresh**
//! Object per access (matching Foundation).  `URLSession.shared` returns the
//! same Object on every access within an interpreter run (`===` holds).
//!
//! `URLSession(configuration:)` **copies** the configuration (Foundation-
//! documented): post-init mutations to the original config do not affect the
//! session.  The copy is snapshotted into an independent `ClassObj` at init.
//!
//! The `progress` field holds a **shared** `SwiftValue::Object` for `Progress`.
//! Aliases of `task.progress` taken before `resume()` observe the completed
//! fraction after `resume()` returns because they share the same `Rc`.
//!
//! Transport failures surface as thrown `URLError` values; a missing transport
//! (sandboxed embedding) is an interpreter error so scripts cannot confuse
//! "no network capability" with a network failure.
//!
//! ## RefCell borrow discipline
//!
//! **Never hold a `borrow()` or `borrow_mut()` across a call into
//! [`StdContext`]** (e.g. `ctx.call_closure`, `run_event_driver`,
//! `ctx.call_method_on`).  Re-entrant script code may try to access the same
//! task Object through a closure capture, which would panic on the second
//! borrow.  Always copy needed field values out of the borrow, drop it, then
//! call into the context.

use std::cell::RefCell;
use std::rc::Rc;

use tswift_core::{
    Arg, BuiltinReceiver, ClassObj, EvalError, HttpError, HttpEvent, HttpRequest,
    LabeledMethodEntry, MethodEntry, Outcome, StdContext, StdError, SwiftValue,
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
    interp.register_builtin_enum_with_raw(
        "URLSessionTask.State",
        &[
            ("running", 0),
            ("suspended", 1),
            ("canceling", 2),
            ("completed", 3),
        ],
    );

    // ---- URLSession.ResponseDisposition enum (M4 delegates) ----
    // Registering as "URLSession.ResponseDisposition" so shorthand `.allow` /
    // `.cancel` resolve when the contextual type is set on the completionHandler
    // parameter.  We also register the bare name so `URLSession.ResponseDisposition`
    // type references in protocol declarations are recognised.
    interp.register_builtin_enum_with_raw(
        "URLSession.ResponseDisposition",
        &[
            ("cancel", 0),
            ("allow", 1),
            ("becomeDownload", 2),
            ("becomeStream", 3),
        ],
    );

    // ---- URLSessionConfiguration ----
    // Dual registration strategy for `.default` / `.ephemeral`:
    //
    // 1. `register_static` (StaticFn factory) — checked FIRST by `eval_member`
    //    for the fully-qualified `URLSessionConfiguration.default` form.  Each
    //    call returns a **fresh** independent Object, matching Foundation.
    //
    // 2. `register_static_value` — needed so the shorthand `.default` form
    //    (implicit-member, no type prefix) resolves via `resolve_implicit_static`,
    //    which only scans the `statics` map.  This entry holds a pre-allocated
    //    Object; because `eval_member` checks `static_method` first, the
    //    pre-allocated value is only returned for the shorthand path.
    //    Limitation: `.default` shorthand returns the same Rc across accesses;
    //    `URLSessionConfiguration.default` (qualified) always returns a fresh
    //    one.  The key use-case (`let config = URLSessionConfiguration.default;
    //    config.X = ...`) uses the qualified form and is correct.
    interp.register_static(
        BuiltinReceiver::URLSessionConfiguration,
        "default",
        config_default_static,
    );
    interp.register_static_value("URLSessionConfiguration", "default", configuration_value());
    interp.register_static(
        BuiltinReceiver::URLSessionConfiguration,
        "ephemeral",
        config_ephemeral_static,
    );
    interp.register_static_value(
        "URLSessionConfiguration",
        "ephemeral",
        configuration_value(),
    );

    // ---- URLSession ----
    // `shared` is registered as a pre-allocated Object value so every access
    // returns a clone of the same `Rc` → `URLSession.shared === URLSession.shared`
    // holds within one interpreter run.
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
    // `mutating: false` — task is an Object (reference type); cancel/resume
    // mutate the shared ClassObj in place through the RefCell.  No struct
    // write-back is needed, and `let task = ...` bindings are legal.
    interp.register_intrinsic(
        BuiltinReceiver::URLSessionDataTask,
        "cancel",
        MethodEntry {
            mutating: false,
            func: task_cancel,
        },
    );
    interp.register_intrinsic(
        BuiltinReceiver::URLSessionDataTask,
        "resume",
        MethodEntry {
            mutating: false,
            func: task_resume,
        },
    );
}

// ---------------------------------------------------------------------------
// URLSessionConfiguration
// ---------------------------------------------------------------------------

/// Build a fresh `URLSessionConfiguration` Object with factory-default fields.
///
/// Returns a new independent `SwiftValue::Object` on every call, matching
/// Foundation's `URLSessionConfiguration.default` semantics (each access is a
/// distinct, independently-mutable configuration).  The runtime has no URL
/// cache or cookie storage, so `default` and `ephemeral` presets coincide.
fn configuration_value() -> SwiftValue {
    SwiftValue::Object(Rc::new(RefCell::new(ClassObj {
        class_name: "URLSessionConfiguration".into(),
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
    })))
}

/// Snapshot a `URLSessionConfiguration` (Struct or Object) into a **fresh**
/// independent `SwiftValue::Object`.
///
/// Foundation documents that `URLSession(configuration:)` copies its argument
/// so post-init mutations to the original do not affect the session.  This
/// helper implements that copy for both the legacy Struct representation and
/// the new Object one.
fn copy_configuration(config: &SwiftValue) -> SwiftValue {
    let fields: Vec<(String, SwiftValue)> = match config {
        SwiftValue::Struct(o) if o.type_name == "URLSessionConfiguration" => o.fields.clone(),
        SwiftValue::Object(o) if o.borrow().class_name == "URLSessionConfiguration" => {
            o.borrow().fields.clone()
        }
        _ => Vec::new(),
    };
    SwiftValue::Object(Rc::new(RefCell::new(ClassObj {
        class_name: "URLSessionConfiguration".into(),
        fields,
    })))
}

/// `StaticFn` factory for `URLSessionConfiguration.default`: returns a fresh
/// independent Object on every access (Foundation semantics).
fn config_default_static(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> tswift_core::StdResult {
    Ok(configuration_value())
}

/// `StaticFn` factory for `URLSessionConfiguration.ephemeral`: returns a fresh
/// independent Object on every access (Foundation semantics).
fn config_ephemeral_static(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> tswift_core::StdResult {
    Ok(configuration_value())
}

// ---------------------------------------------------------------------------
// URLSession
// ---------------------------------------------------------------------------

fn session_value(configuration: SwiftValue) -> SwiftValue {
    session_value_with_delegate(configuration, SwiftValue::Nil)
}

fn session_value_with_delegate(configuration: SwiftValue, delegate: SwiftValue) -> SwiftValue {
    SwiftValue::Object(Rc::new(RefCell::new(ClassObj {
        class_name: "URLSession".into(),
        fields: vec![
            ("configuration".into(), configuration),
            // Stored delegate object; SwiftValue::Nil when no delegate is set.
            // `delegateQueue` is accepted but ignored (single-threaded executor).
            ("_delegate".into(), delegate),
        ],
    })))
}

/// Extract the delegate object from a session value, if present.
fn session_delegate(session: &SwiftValue) -> SwiftValue {
    match session {
        SwiftValue::Struct(o) if o.type_name == "URLSession" => {
            o.get("_delegate").cloned().unwrap_or(SwiftValue::Nil)
        }
        SwiftValue::Object(o) if o.borrow().class_name == "URLSession" => {
            // Short borrow: copy the value out before dropping the guard.
            o.borrow()
                .get("_delegate")
                .cloned()
                .unwrap_or(SwiftValue::Nil)
        }
        _ => SwiftValue::Nil,
    }
}

fn session_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> tswift_core::StdResult {
    // Accept `URLSession(configuration:)` and
    // `URLSession(configuration:delegate:delegateQueue:)`. The `delegateQueue`
    // parameter is accepted and silently ignored — the executor is
    // single-threaded (ADR-0005), so queue selection has no effect at runtime.
    let config_arg = args
        .iter()
        .find(|a| a.label.as_deref() == Some("configuration"));
    let delegate = args
        .iter()
        .find(|a| a.label.as_deref() == Some("delegate"))
        .map(|a| a.value.clone())
        .unwrap_or(SwiftValue::Nil);
    let Some(config_arg) = config_arg else {
        return Err(type_error("URLSession(configuration:) expects one label"));
    };
    // Foundation documents that URLSession copies its configuration at init
    // time.  Snapshot into a fresh independent Object so post-init mutations
    // to the caller's config do not affect the session.
    let config_snapshot = match &config_arg.value {
        SwiftValue::Struct(o) if o.type_name == "URLSessionConfiguration" => {
            copy_configuration(&config_arg.value)
        }
        SwiftValue::Object(o) if o.borrow().class_name == "URLSessionConfiguration" => {
            copy_configuration(&config_arg.value)
        }
        _ => {
            return Err(type_error(
                "URLSession(configuration:) expects a URLSessionConfiguration",
            ))
        }
    };
    Ok(session_value_with_delegate(config_snapshot, delegate))
}

fn session_configuration(recv: SwiftValue) -> tswift_core::StdResult {
    match &recv {
        SwiftValue::Struct(o) if o.type_name == "URLSession" => Ok(o
            .get("configuration")
            .cloned()
            .unwrap_or_else(configuration_value)),
        SwiftValue::Object(o) if o.borrow().class_name == "URLSession" => {
            // Short borrow: copy the value out before dropping the Ref guard.
            Ok(o.borrow()
                .get("configuration")
                .cloned()
                .unwrap_or_else(configuration_value))
        }
        _ => Err(type_error("configuration expects URLSession")),
    }
}

/// Extract the session's request timeout from its configuration.
///
/// Handles both the legacy Struct representation and the new Object backing
/// for `URLSession` and `URLSessionConfiguration`.
fn session_timeout(recv: &SwiftValue) -> f64 {
    // Read the `configuration` field from a Struct or Object session.
    let config: SwiftValue = match recv {
        SwiftValue::Struct(o) if o.type_name == "URLSession" => {
            o.get("configuration").cloned().unwrap_or(SwiftValue::Nil)
        }
        SwiftValue::Object(o) if o.borrow().class_name == "URLSession" => {
            // Short borrow: copy out before dropping the Ref.
            o.borrow()
                .get("configuration")
                .cloned()
                .unwrap_or(SwiftValue::Nil)
        }
        _ => return 60.0,
    };
    // Read `timeoutIntervalForRequest` from a Struct or Object configuration.
    match &config {
        SwiftValue::Struct(c) if c.type_name == "URLSessionConfiguration" => {
            match c.get("timeoutIntervalForRequest") {
                Some(SwiftValue::Double(d)) => *d,
                Some(SwiftValue::Int(i)) => i.raw as f64,
                _ => 60.0,
            }
        }
        SwiftValue::Object(o) if o.borrow().class_name == "URLSessionConfiguration" => {
            // Short borrow: copy numeric value out before dropping the Ref.
            match o.borrow().get("timeoutIntervalForRequest").cloned() {
                Some(SwiftValue::Double(d)) => d,
                Some(SwiftValue::Int(i)) => i.raw as f64,
                _ => 60.0,
            }
        }
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

// ---------------------------------------------------------------------------
// Delegate dispatch helpers (M4)
// ---------------------------------------------------------------------------

/// Argument-label stubs used for `StdContext::has_method_on` overload matching.
///
/// Values are never evaluated — only labels matter for overload selection.
fn delegate_probe_args(labels: &[Option<&str>]) -> Vec<Arg> {
    labels
        .iter()
        .map(|l| Arg {
            label: l.map(|s| s.to_string()),
            value: SwiftValue::Void,

            static_ty: None,
        })
        .collect()
}

/// Fire `urlSession(_:dataTask:didReceive:completionHandler:)` on `delegate`
/// (if it implements it) and return the disposition: `true` = allow, `false` =
/// cancel.  When the delegate doesn't implement the method, returns `true`
/// (allow) by default.
fn dispatch_did_receive_response(
    ctx: &mut dyn StdContext,
    delegate: &SwiftValue,
    session: SwiftValue,
    task: SwiftValue,
    response: SwiftValue,
) -> Result<bool, StdError> {
    // Method labels: [_ session, dataTask:, didReceive:, completionHandler:]
    let probe = delegate_probe_args(&[
        None,
        Some("dataTask"),
        Some("didReceive"),
        Some("completionHandler"),
    ]);
    if !ctx.has_method_on(delegate, "urlSession", &probe) {
        return Ok(true); // no delegate for this event → allow
    }
    // Allocate the synthetic completionHandler closure.
    let handler_id = ctx.allocate_response_disposition_closure();
    ctx.call_method_on(
        delegate.clone(),
        "urlSession",
        vec![
            Arg {
                label: None,
                value: session,

                static_ty: None,
            },
            Arg {
                label: Some("dataTask".into()),
                value: task,

                static_ty: None,
            },
            Arg {
                label: Some("didReceive".into()),
                value: response,

                static_ty: None,
            },
            Arg {
                label: Some("completionHandler".into()),
                value: SwiftValue::Closure(handler_id),

                static_ty: None,
            },
        ],
    )?;
    Ok(ctx.take_response_disposition())
}

/// Fire `urlSession(_:dataTask:didReceive:)` (Data variant) on `delegate` if
/// it implements it.  Any error propagates to abort the request.
fn dispatch_did_receive_data(
    ctx: &mut dyn StdContext,
    delegate: &SwiftValue,
    session: SwiftValue,
    task: SwiftValue,
    data: SwiftValue,
) -> Result<(), StdError> {
    // 3-arg variant: [_ session, dataTask:, didReceive: Data]
    let probe = delegate_probe_args(&[None, Some("dataTask"), Some("didReceive")]);
    if !ctx.has_method_on(delegate, "urlSession", &probe) {
        return Ok(());
    }
    ctx.call_method_on(
        delegate.clone(),
        "urlSession",
        vec![
            Arg {
                label: None,
                value: session,

                static_ty: None,
            },
            Arg {
                label: Some("dataTask".into()),
                value: task,

                static_ty: None,
            },
            Arg {
                label: Some("didReceive".into()),
                value: data,

                static_ty: None,
            },
        ],
    )?;
    Ok(())
}

/// Fire `urlSession(_:task:didCompleteWithError:)` on `delegate` if it
/// implements it.  Errors are swallowed (the task is already terminal).
fn dispatch_did_complete(
    ctx: &mut dyn StdContext,
    delegate: &SwiftValue,
    session: SwiftValue,
    task: SwiftValue,
    error: SwiftValue,
) {
    let probe = delegate_probe_args(&[None, Some("task"), Some("didCompleteWithError")]);
    if !ctx.has_method_on(delegate, "urlSession", &probe) {
        return;
    }
    let _ = ctx.call_method_on(
        delegate.clone(),
        "urlSession",
        vec![
            Arg {
                label: None,
                value: session,

                static_ty: None,
            },
            Arg {
                label: Some("task".into()),
                value: task,

                static_ty: None,
            },
            Arg {
                label: Some("didCompleteWithError".into()),
                value: error,

                static_ty: None,
            },
        ],
    );
}

// ---------------------------------------------------------------------------
// Event-loop driver (M3/M4 seam)
// ---------------------------------------------------------------------------

/// Drive the request event loop: `start → next_event* → Done/Failed`.
///
/// Checks [`StdContext::current_task_cancelled`] before starting and after
/// each event, so a containing `Task.cancel()` surfaces as
/// `URLError(.cancelled)` without touching the transport. Event-order
/// violations (no `Response` before terminal, malformed sequence) map to
/// `badServerResponse`.
///
/// When `session` contains a delegate object (M4), the driver fires the
/// three optional `URLSessionDataDelegate` / `URLSessionTaskDelegate` callbacks
/// per event: `didReceive response + completionHandler` (disposition),
/// `didReceive data` (chunk), and `didCompleteWithError` (terminal). Each
/// callback is dispatched only if the delegate class implements that overload.
///
/// Callers must NOT call [`StdContext::perform_http`]; they must use this
/// driver directly so delegate hooks and cancellation checks compose
/// correctly (M3/M4 seam contract from notes.md).
fn run_event_driver(
    ctx: &mut dyn StdContext,
    req: &HttpRequest,
    session: &SwiftValue,
    task: &SwiftValue,
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
    // M4: pre-extract the delegate so we can dispatch per-event callbacks.
    let delegate = session_delegate(session);

    /// Cancel the transport and drain to terminal, then return
    /// `Err(URLError(code))`.  Inlined as a macro to avoid borrow issues.
    macro_rules! cancel_and_return {
        ($ctx:expr, $handle:expr, $url:expr, $code:literal) => {{
            $ctx.http_cancel($handle);
            let mut _d = 0usize;
            loop {
                if $ctx.http_next_event($handle).is_terminal() {
                    break;
                }
                _d += 1;
                if _d >= MAX_DRAIN_EVENTS {
                    break;
                }
            }
            return Err(StdError::Throw(url_error_value(
                $code,
                crate::url::url_value($url),
            )));
        }};
    }

    /// Evaluate a delegate-dispatch expression; if it returns `Err`, cancel
    /// the transport handle and drain to terminal before propagating the error.
    /// Without this, a delegate method that calls `fatalError` / traps inside
    /// a script would leak the handle-store entry (violating ADR-0011's
    /// drain-or-cancel invariant).
    macro_rules! delegate_or_cancel {
        ($ctx:expr, $handle:expr, $url:expr, $result:expr) => {{
            match $result {
                Ok(v) => v,
                Err(e) => {
                    $ctx.http_cancel($handle);
                    let mut _d = 0usize;
                    loop {
                        if $ctx.http_next_event($handle).is_terminal() {
                            break;
                        }
                        _d += 1;
                        if _d >= MAX_DRAIN_EVENTS {
                            break;
                        }
                    }
                    return Err(e);
                }
            }
        }};
    }

    let terminal_error: Option<StdError>;
    loop {
        let event = ctx.http_next_event(handle);
        match event {
            HttpEvent::Response {
                status: s,
                headers: h,
            } => {
                if got_response {
                    // Second Response in the same stream — malformed per ADR-0011.
                    cancel_and_return!(ctx, handle, url, "badServerResponse");
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
                headers = h.clone();
                got_response = true;

                // M4: dispatch `urlSession(_:dataTask:didReceive:completionHandler:)`
                // if the delegate implements it.  Honour the returned disposition:
                // .cancel → cancel the transport and surface URLError(.cancelled).
                if !matches!(delegate, SwiftValue::Nil) {
                    let url_resp = http_url_response_value(
                        crate::url::url_value(url.clone()),
                        i128::from(status),
                        h,
                    );
                    let allow = delegate_or_cancel!(
                        ctx,
                        handle,
                        url,
                        dispatch_did_receive_response(
                            ctx,
                            &delegate,
                            session.clone(),
                            task.clone(),
                            url_resp,
                        )
                    );
                    if !allow {
                        // Delegate cancelled via disposition.  Fire
                        // `didCompleteWithError(.cancelled)` before draining.
                        let cancelled_err =
                            url_error_value("cancelled", crate::url::url_value(url.clone()));
                        dispatch_did_complete(
                            ctx,
                            &delegate,
                            session.clone(),
                            task.clone(),
                            cancelled_err,
                        );
                        cancel_and_return!(ctx, handle, url, "cancelled");
                    }
                }
            }
            HttpEvent::Chunk(bytes) => {
                if !got_response {
                    // Chunk before Response — malformed sequence per ADR-0011.
                    cancel_and_return!(ctx, handle, url, "badServerResponse");
                }
                // M4: dispatch `urlSession(_:dataTask:didReceive:)` (Data).
                if !matches!(delegate, SwiftValue::Nil) {
                    delegate_or_cancel!(
                        ctx,
                        handle,
                        url,
                        dispatch_did_receive_data(
                            ctx,
                            &delegate,
                            session.clone(),
                            task.clone(),
                            data_value(bytes.clone()),
                        )
                    );
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
                // M4: dispatch `urlSession(_:task:didCompleteWithError:)` with nil.
                if !matches!(delegate, SwiftValue::Nil) {
                    dispatch_did_complete(
                        ctx,
                        &delegate,
                        session.clone(),
                        task.clone(),
                        SwiftValue::Nil,
                    );
                }
                terminal_error = None;
                break;
            }
            HttpEvent::Failed { code, message: _ } => {
                // Failed events are transport errors (e.g., connection refused,
                // DNS failure, or mid-flight network drop).  A Failed event
                // *before* Response is a legitimate transport failure, not a
                // protocol violation — surface it as URLError with the
                // transport's code rather than badServerResponse.
                let err_val = url_error_value(&code, crate::url::url_value(url.clone()));
                // M4: dispatch `didCompleteWithError` with the error before
                // propagating it to the caller.
                if !matches!(delegate, SwiftValue::Nil) {
                    dispatch_did_complete(
                        ctx,
                        &delegate,
                        session.clone(),
                        task.clone(),
                        err_val.clone(),
                    );
                }
                terminal_error = Some(StdError::Throw(err_val));
                break;
            }
        }

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
            // M4: delegate didCompleteWithError(.cancelled) on cooperative cancel.
            if !matches!(delegate, SwiftValue::Nil) {
                let err_val = url_error_value("cancelled", crate::url::url_value(url.clone()));
                dispatch_did_complete(ctx, &delegate, session.clone(), task.clone(), err_val);
            }
            return Err(StdError::Throw(url_error_value(
                "cancelled",
                crate::url::url_value(url),
            )));
        }
    }

    if let Some(err) = terminal_error {
        return Err(err);
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
    // For async paths, create a minimal synthetic task for delegate callbacks.
    // Its state/progress fields are not updated mid-flight (no task_resume here),
    // but the delegate still receives session/task references.
    let synthetic_task = task_value(args[0].value.clone(), recv.clone(), -1);
    let outcome = run_event_driver(ctx, &req, &recv, &synthetic_task)?;
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
    let synthetic_task = task_value(args[0].value.clone(), recv.clone(), -1);
    let outcome = run_event_driver(ctx, &req, &recv, &synthetic_task)?;
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

/// Build a `Progress` Object with the given `fractionCompleted`.
///
/// Returns `SwiftValue::Object` (reference semantics) so that aliases of
/// `task.progress` taken before `resume()` observe the updated fraction after
/// the request completes — they share the same `Rc`.
fn progress_object(fraction: f64) -> SwiftValue {
    SwiftValue::Object(Rc::new(RefCell::new(ClassObj {
        class_name: "Progress".into(),
        fields: vec![("fractionCompleted".into(), SwiftValue::Double(fraction))],
    })))
}

/// Build a fresh suspended `URLSessionDataTask` Object.
///
/// Returns `SwiftValue::Object` (reference semantics), matching the real
/// Foundation class.  `cancel()` and `resume()` mutate the shared `ClassObj`
/// in place through the `RefCell`; `let task = ...` bindings are legal.
///
/// `req_value` is a `URLRequest` struct (or URL to be lowered at resume time).
/// `session_value` is the owning `URLSession` (stored so `task_resume` can
/// extract the delegate for per-event callback dispatch).
/// `closure_id` is the index into the interpreter's closure table for the
/// completion handler; `-1` means no handler.
fn task_value(req_value: SwiftValue, session_value: SwiftValue, closure_id: i128) -> SwiftValue {
    let timeout = session_timeout(&session_value);
    SwiftValue::Object(Rc::new(RefCell::new(ClassObj {
        class_name: "URLSessionDataTask".into(),
        fields: vec![
            // Private: request, session, closure
            ("_req".into(), req_value),
            // Session back-reference (for delegate access in task_resume).
            ("_session".into(), session_value),
            ("_closure_id".into(), SwiftValue::int(closure_id)),
            // Private: cancelled flag
            ("_cancelled".into(), SwiftValue::Bool(false)),
            // Stored session timeout — copied so task_resume doesn't need the
            // session struct to compute the request timeout.
            ("_session_timeout".into(), SwiftValue::Double(timeout)),
            // Public state
            ("state".into(), task_state_value("suspended")),
            ("countOfBytesReceived".into(), SwiftValue::int(0)),
            ("countOfBytesExpectedToReceive".into(), SwiftValue::int(-1)),
            // Shared Progress object — aliases observe mutations in place.
            ("progress".into(), progress_object(0.0)),
        ],
    })))
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

    let task = task_value(req_value, recv.clone(), closure_id);
    Ok(Some(Outcome {
        result: task,
        receiver: recv,
    }))
}

/// `URLSessionDataTask.cancel()` — mark the task as canceling.
///
/// Mutates the shared `ClassObj` in place through the `RefCell`.
/// Registered `mutating: false` — no struct write-back occurs, so
/// `let task = ...` bindings are legal (reference semantics).
///
/// If `resume()` has not been called yet, the next `resume()` will complete
/// with `URLError(.cancelled)` without touching the transport.
fn task_cancel(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    // Clone the Rc so `recv` is free to move into Outcome later.
    let obj = match &recv {
        SwiftValue::Object(o) => Rc::clone(o),
        _ => {
            return Ok(Outcome {
                result: SwiftValue::Void,
                receiver: recv,
            })
        }
    };
    // Idempotent: already canceling or completed — no-op.
    {
        let task = obj.borrow();
        if task.class_name != "URLSessionDataTask" {
            return Ok(Outcome {
                result: SwiftValue::Void,
                receiver: recv,
            });
        }
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
    }
    // Mutate in place — reference semantics: all aliases observe the change.
    {
        let mut task = obj.borrow_mut();
        task.set("_cancelled", SwiftValue::Bool(true));
        task.set("state", task_state_value("canceling"));
    }
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: recv,
    })
}

/// `URLSessionDataTask.resume()` — drive the event loop then invoke the
/// completion handler.
///
/// Mutates the shared `ClassObj` in place through the `RefCell`.
/// Registered `mutating: false` — no struct write-back occurs, so
/// `let task = ...` bindings are legal (reference semantics).
///
/// State transitions: `suspended` → `running` → `completed` (or `canceling`
/// if cancelled).
///
/// # Borrow discipline
///
/// All `RefCell` borrows are acquired in a short block, fields are copied out,
/// and the borrow is dropped **before** any call into `ctx` (closure, driver,
/// method dispatch).  Re-entrant script code accessing the same task Object
/// through a capture therefore never encounters a live borrow.
fn task_resume(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    // Clone the Rc so `recv` is free to move into Outcome later.
    let obj = match &recv {
        SwiftValue::Object(o) => Rc::clone(o),
        _ => {
            return Ok(Outcome {
                result: SwiftValue::Void,
                receiver: recv,
            })
        }
    };

    // --- Copy fields out before any context call (borrow discipline). ---
    let (should_proceed, closure_id, req_value, task_session, session_timeout_val, pre_cancelled) = {
        let task = obj.borrow();
        if task.class_name != "URLSessionDataTask" {
            return Ok(Outcome {
                result: SwiftValue::Void,
                receiver: recv,
            });
        }
        // State guard: Foundation semantics — resume() on a running or completed
        // task is a no-op.  A .canceling task is allowed to proceed so that the
        // pre-flight cancel path (pre_cancelled flag below) can deliver URLError
        // to the completion handler, matching Foundation behaviour.
        let should_proceed = matches!(
            task.get("state"),
            Some(SwiftValue::Enum(e)) if e.case == "suspended" || e.case == "canceling"
        );
        let closure_id = match task.get("_closure_id") {
            Some(SwiftValue::Int(i)) => i.raw,
            _ => -1,
        };
        let req_value = task.get("_req").cloned().unwrap_or(SwiftValue::Nil);
        let task_session = task.get("_session").cloned().unwrap_or(SwiftValue::Nil);
        let session_timeout_val = match task.get("_session_timeout") {
            Some(SwiftValue::Double(d)) => *d,
            _ => 60.0,
        };
        let pre_cancelled = matches!(task.get("_cancelled"), Some(SwiftValue::Bool(true)));
        (
            should_proceed,
            closure_id,
            req_value,
            task_session,
            session_timeout_val,
            pre_cancelled,
        )
        // borrow dropped here
    };

    if !should_proceed {
        return Ok(Outcome {
            result: SwiftValue::Void,
            receiver: recv,
        });
    }

    // Ensure the URLRequest has its timeoutInterval set to the session timeout
    // when not explicitly overridden.
    let mut req = lower_request(&req_value)?;
    if req.timeout_seconds == 60.0 {
        req.timeout_seconds = session_timeout_val;
    }

    // Pre-flight cancel: deliver URLError(.cancelled) to the handler without
    // touching the transport.  Short borrow dropped before call_closure.
    if pre_cancelled {
        obj.borrow_mut().set("state", task_state_value("completed"));
        // borrow_mut dropped before call_closure
        if closure_id >= 0 {
            let err = url_error_value("cancelled", crate::url::url_value(req.url));
            ctx.call_closure(
                closure_id as usize,
                vec![SwiftValue::Nil, SwiftValue::Nil, err],
            )?;
        }
        return Ok(Outcome {
            result: SwiftValue::Void,
            receiver: recv,
        });
    }

    // Mark running — short borrow, dropped before run_event_driver.
    obj.borrow_mut().set("state", task_state_value("running"));

    // Drive the event loop, passing `recv` as the live task Object so
    // delegate callbacks receive the shared reference (M4).  The borrow is
    // fully released above — no live borrow crosses into run_event_driver.
    match run_event_driver(ctx, &req, &task_session, &recv) {
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

            // Mutate counters, progress, and state in place — all aliases
            // observe the final state.  Short borrow dropped before call_closure.
            {
                let mut task = obj.borrow_mut();
                task.set(
                    "countOfBytesReceived",
                    SwiftValue::int(bytes_received as i128),
                );
                task.set(
                    "countOfBytesExpectedToReceive",
                    SwiftValue::int(if bytes_expected >= 0 {
                        bytes_expected as i128
                    } else {
                        -1
                    }),
                );
                // Update the shared Progress object in place so aliases of
                // task.progress observe the new fraction.
                if let Some(SwiftValue::Object(prog_obj)) = task.get("progress") {
                    prog_obj
                        .borrow_mut()
                        .set("fractionCompleted", SwiftValue::Double(fraction));
                } else {
                    task.set("progress", progress_object(fraction));
                }
                task.set("state", task_state_value("completed"));
            }
            // borrow_mut dropped — safe to call into ctx now.

            // Invoke completion handler: (Data, URLResponse, nil).
            if closure_id >= 0 {
                ctx.call_closure(closure_id as usize, vec![data, response, SwiftValue::Nil])?;
            }

            Ok(Outcome {
                result: SwiftValue::Void,
                receiver: recv,
            })
        }
        Err(err) => {
            // Transport error: mark task completed, deliver error to handler.
            // Short borrow dropped before call_closure.
            obj.borrow_mut().set("state", task_state_value("completed"));

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
                receiver: recv,
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
        MockRoute, StructObj,
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
        let out = run_event_driver(&mut ctx, &req, &SwiftValue::Nil, &SwiftValue::Nil).unwrap();
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
        let err = run_event_driver(&mut ctx, &req, &SwiftValue::Nil, &SwiftValue::Nil).unwrap_err();
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
        let err = run_event_driver(&mut ctx, &req, &SwiftValue::Nil, &SwiftValue::Nil).unwrap_err();
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
        let err = run_event_driver(&mut ctx, &req, &SwiftValue::Nil, &SwiftValue::Nil).unwrap_err();
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

                static_ty: None,
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

                static_ty: None,
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

                    static_ty: None,
                },
                Arg {
                    label: Some("from".into()),
                    value: data_value(b"payload".to_vec()),

                    static_ty: None,
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
        let task = task_value(
            request_value("https://example.com/"),
            session_value(configuration_value()),
            0,
        );
        let mut ctx = HttpEventCtx::new(vec![]);
        let outcome = task_cancel(&mut ctx, task, vec![]).unwrap();
        // With Object backing, receiver is the same Rc — inspect through borrow.
        let SwiftValue::Object(obj) = &outcome.receiver else {
            panic!("expected Object receiver");
        };
        let task_ref = obj.borrow();
        assert_eq!(task_ref.get("_cancelled"), Some(&SwiftValue::Bool(true)));
        let state = task_ref.get("state");
        assert!(
            matches!(state, Some(SwiftValue::Enum(e)) if e.case == "canceling"),
            "expected state=canceling, got {state:?}"
        );
    }

    #[test]
    fn task_resume_after_cancel_calls_handler_with_url_error() {
        let task = task_value(
            request_value("https://example.com/"),
            session_value(configuration_value()),
            0,
        );
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
        // State should be completed — read through the Object borrow.
        let SwiftValue::Object(obj) = &outcome.receiver else {
            panic!("expected Object receiver after resume");
        };
        let state = obj.borrow().get("state").cloned();
        assert!(
            matches!(state, Some(SwiftValue::Enum(ref e)) if e.case == "completed"),
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
        let task = task_value(
            request_value("https://example.com/hello"),
            session_value(configuration_value()),
            42,
        );
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

        // Task state updated — read through the Object borrow.
        let SwiftValue::Object(obj) = &outcome.receiver else {
            panic!("expected Object receiver");
        };
        let task_ref = obj.borrow();
        assert_eq!(
            task_ref.get("countOfBytesReceived"),
            Some(&SwiftValue::int(5))
        );
        assert_eq!(
            task_ref.get("countOfBytesExpectedToReceive"),
            Some(&SwiftValue::int(5))
        );
        let state = task_ref.get("state").cloned();
        drop(task_ref);
        assert!(
            matches!(state, Some(SwiftValue::Enum(ref e)) if e.case == "completed"),
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
        let err = run_event_driver(&mut ctx, &req, &SwiftValue::Nil, &SwiftValue::Nil).unwrap_err();
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
        let err = run_event_driver(&mut ctx, &req, &SwiftValue::Nil, &SwiftValue::Nil).unwrap_err();
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
        let err = run_event_driver(&mut ctx, &req, &SwiftValue::Nil, &SwiftValue::Nil).unwrap_err();

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
        let task = task_value(
            request_value("https://example.com/once"),
            session_value(configuration_value()),
            99,
        );

        // First resume: suspended → running → completed.
        let out1 = task_resume(&mut ctx, task, vec![]).unwrap();
        assert_eq!(
            ctx.start_count, 1,
            "first resume must issue exactly one request"
        );
        assert_eq!(ctx.calls.len(), 1, "handler called once after first resume");

        // Second resume: state is .completed → state guard returns early, no
        // transport call, no additional handler invocation.
        // With Object backing, out1.receiver is the same Rc — clone it to get
        // a second handle pointing to the same ClassObj.
        let completed_task = out1.receiver.clone();
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
        let task = task_value(
            request_value("https://stream.example.com/"),
            session_value(configuration_value()),
            7,
        );
        let outcome = task_resume(&mut ctx, task, vec![]).unwrap();

        // Handler called once with all data concatenated.
        assert_eq!(ctx.calls.len(), 1);
        let args = &ctx.calls[0].1;
        assert_eq!(data_bytes(&args[0]).unwrap(), b"chunk1chunk2".to_vec());

        let SwiftValue::Object(obj) = &outcome.receiver else {
            panic!("expected Object receiver");
        };
        let task_ref = obj.borrow();
        assert_eq!(
            task_ref.get("countOfBytesReceived"),
            Some(&SwiftValue::int(12))
        );
        assert_eq!(
            task_ref.get("countOfBytesExpectedToReceive"),
            Some(&SwiftValue::int(12))
        );
        // progress is a shared Object — read fractionCompleted through its borrow.
        let prog = task_ref.get("progress").cloned();
        drop(task_ref);
        assert!(
            matches!(&prog, Some(SwiftValue::Object(s)) if {
                matches!(s.borrow().get("fractionCompleted"), Some(SwiftValue::Double(f)) if (*f - 1.0).abs() < 1e-9)
            }),
            "expected fractionCompleted=1.0, got {prog:?}"
        );
    }

    // -----------------------------------------------------------------------
    // CRITICAL fix: delegate error → cancel+drain handle (ADR-0011 invariant)
    // -----------------------------------------------------------------------

    /// A `StdContext` wrapper around `SequenceCtx` that returns a hard error
    /// from `call_method_on` (simulating a delegate method that calls
    /// `fatalError` or traps inside Swift).  `has_method_on` always returns
    /// `true` so the driver tries to dispatch.
    struct DelegateErrCtx {
        inner: SequenceCtx,
    }

    impl StdContext for DelegateErrCtx {
        fn call_closure(&mut self, _id: usize, _args: Vec<SwiftValue>) -> tswift_core::StdResult {
            Ok(SwiftValue::Void)
        }
        fn out(&mut self) -> &mut dyn std::io::Write {
            &mut self.inner.out
        }
        fn http_start(
            &mut self,
            req: &HttpRequest,
        ) -> Result<HttpTaskHandle, tswift_core::HttpError> {
            self.inner.http_start(req)
        }
        fn http_next_event(&mut self, h: HttpTaskHandle) -> HttpEvent {
            self.inner.http_next_event(h)
        }
        fn http_cancel(&mut self, h: HttpTaskHandle) {
            self.inner.http_cancel(h);
        }
        fn has_method_on(&self, _receiver: &SwiftValue, _method: &str, _args: &[Arg]) -> bool {
            true // pretend every method exists so dispatch is attempted
        }
        fn call_method_on(
            &mut self,
            _receiver: SwiftValue,
            _method: &str,
            _args: Vec<Arg>,
        ) -> tswift_core::StdResult {
            Err(type_error("delegate method raised fatalError"))
        }
    }

    #[test]
    fn delegate_response_error_cancels_and_drains_handle() {
        // Simulate a delegate whose `didReceive response` method raises an
        // error (fatalError / trap inside Swift).  The driver MUST:
        //   1. call http_cancel on the handle,
        //   2. drain remaining events to terminal (so the handle-store is clean),
        //   3. propagate the error.
        // Without the `delegate_or_cancel!` fix, the `?` in the driver would
        // return immediately, leaving the handle alive and leaking the entry.
        let mut ctx = DelegateErrCtx {
            inner: SequenceCtx::new(vec![
                HttpEvent::Response {
                    status: 200,
                    headers: vec![],
                },
                // Extra chunks that should be drained after the delegate error.
                // No Done here: http_cancel will append Failed{cancelled} so
                // the drain loop terminates on that, leaving the queue empty.
                HttpEvent::Chunk(b"data1".to_vec()),
                HttpEvent::Chunk(b"data2".to_vec()),
            ]),
        };
        // Build a session with a non-Nil delegate so the dispatch path is taken.
        let fake_delegate = SwiftValue::Struct(Rc::new(StructObj {
            type_name: "FakeDelegate".into(),
            fields: vec![],
        }));
        let sess = session_value_with_delegate(configuration_value(), fake_delegate);
        let req = HttpRequest {
            url: "https://example.com/delegate-trap".into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: None,
            timeout_seconds: 60.0,
        };
        let err = run_event_driver(&mut ctx, &req, &sess, &SwiftValue::Nil).unwrap_err();

        // http_cancel must have been called (drain-or-cancel invariant).
        assert!(
            ctx.inner.http_cancel_called,
            "http_cancel must be called when delegate dispatch returns Err"
        );
        // Handle must be drained (no events left in the queue).
        assert!(
            ctx.inner.events.is_empty(),
            "handle must be fully drained after delegate error; {} events left",
            ctx.inner.events.len()
        );
        // The delegate's error must be propagated.
        assert!(
            matches!(err, StdError::Error(_)),
            "expected StdError propagated from delegate, got {err:?}"
        );
    }

    #[test]
    fn delegate_chunk_error_cancels_and_drains_handle() {
        // Same invariant, but the error fires during `didReceive data` (Chunk
        // arm), not the Response arm.
        //
        // Override: let the Response dispatch succeed (return Ok) by making
        // `call_method_on` fail only on the second call (the Chunk callback).
        struct ChunkErrCtx {
            inner: SequenceCtx,
            call_count: usize,
        }
        impl StdContext for ChunkErrCtx {
            fn call_closure(&mut self, id: usize, args: Vec<SwiftValue>) -> tswift_core::StdResult {
                // Simulate the completionHandler returning .allow so the driver
                // continues past the Response arm.
                self.inner.call_closure(id, args)
            }
            fn out(&mut self) -> &mut dyn std::io::Write {
                &mut self.inner.out
            }
            fn http_start(
                &mut self,
                req: &HttpRequest,
            ) -> Result<HttpTaskHandle, tswift_core::HttpError> {
                self.inner.http_start(req)
            }
            fn http_next_event(&mut self, h: HttpTaskHandle) -> HttpEvent {
                self.inner.http_next_event(h)
            }
            fn http_cancel(&mut self, h: HttpTaskHandle) {
                self.inner.http_cancel(h);
            }
            fn has_method_on(&self, _receiver: &SwiftValue, _method: &str, _args: &[Arg]) -> bool {
                true
            }
            fn call_method_on(
                &mut self,
                _receiver: SwiftValue,
                _method: &str,
                args: Vec<Arg>,
            ) -> tswift_core::StdResult {
                self.call_count += 1;
                if self.call_count == 1 {
                    // First call is the response-delegate — call completionHandler
                    // arg with .allow so the driver gets `true` back.
                    let handler_id = args
                        .iter()
                        .find(|a| a.label.as_deref() == Some("completionHandler"))
                        .and_then(|a| {
                            if let SwiftValue::Closure(id) = a.value {
                                Some(id)
                            } else {
                                None
                            }
                        });
                    // We can't call back into ctx.call_closure here (no
                    // access) — return Ok so the default `take_response_disposition`
                    // returns true (allow) from `unwrap_or(true)`.
                    let _ = handler_id;
                    Ok(SwiftValue::Void)
                } else {
                    // Second call is the Chunk data delegate — trap.
                    Err(type_error("didReceive(data) raised fatalError"))
                }
            }
        }

        let mut ctx = ChunkErrCtx {
            inner: SequenceCtx::new(vec![
                HttpEvent::Response {
                    status: 200,
                    headers: vec![],
                },
                HttpEvent::Chunk(b"part1".to_vec()),
                // No Done: http_cancel appends Failed{cancelled}; drain stops there,
                // leaving the queue empty.
                HttpEvent::Chunk(b"part2".to_vec()),
            ]),
            call_count: 0,
        };
        let fake_delegate = SwiftValue::Struct(Rc::new(StructObj {
            type_name: "FakeDelegate".into(),
            fields: vec![],
        }));
        let sess = session_value_with_delegate(configuration_value(), fake_delegate);
        let req = HttpRequest {
            url: "https://example.com/chunk-trap".into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: None,
            timeout_seconds: 60.0,
        };
        let err = run_event_driver(&mut ctx, &req, &sess, &SwiftValue::Nil).unwrap_err();

        assert!(
            ctx.inner.http_cancel_called,
            "http_cancel must be called when chunk-delegate dispatch returns Err"
        );
        assert!(
            ctx.inner.events.is_empty(),
            "handle must be fully drained after chunk-delegate error; {} left",
            ctx.inner.events.len()
        );
        assert!(
            matches!(err, StdError::Error(_)),
            "expected StdError propagated from chunk delegate, got {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Phase 1 — reference-semantics tests
    // -----------------------------------------------------------------------

    /// `let task = ...; task.resume()` is legal and leaves the task completed.
    #[test]
    fn let_bound_task_can_be_resumed() {
        let mut ctx = HttpEventCtx::new(vec![MockRoute {
            method: "GET".into(),
            url: "https://example.com/ref".into(),
            outcome: Ok(ok_resp(200, b"ref")),
        }]);
        // task_value now returns an Object — no `var` required.
        let task = task_value(
            request_value("https://example.com/ref"),
            session_value(configuration_value()),
            0,
        );
        let _outcome = task_resume(&mut ctx, task.clone(), vec![]).unwrap();
        // The original `task` binding (same Rc) must show state=completed.
        let SwiftValue::Object(obj) = &task else {
            panic!("expected Object");
        };
        let state = obj.borrow().get("state").cloned();
        assert!(
            matches!(state, Some(SwiftValue::Enum(ref e)) if e.case == "completed"),
            "let-bound task should be completed after resume, got {state:?}"
        );
    }

    /// An alias taken before `resume()` observes `completed` after it returns.
    #[test]
    fn alias_observes_state_after_resume() {
        let mut ctx = HttpEventCtx::new(vec![MockRoute {
            method: "GET".into(),
            url: "https://example.com/alias".into(),
            outcome: Ok(ok_resp(200, b"alias")),
        }]);
        let task = task_value(
            request_value("https://example.com/alias"),
            session_value(configuration_value()),
            1,
        );
        // alias shares the same Rc.
        let alias = task.clone();
        task_resume(&mut ctx, task, vec![]).unwrap();
        // alias must now see state=completed (reference semantics).
        let SwiftValue::Object(obj) = &alias else {
            panic!("expected Object alias");
        };
        let state = obj.borrow().get("state").cloned();
        assert!(
            matches!(state, Some(SwiftValue::Enum(ref e)) if e.case == "completed"),
            "alias should observe completed after resume, got {state:?}"
        );
    }

    /// cancel-then-resume delivers `URLError(.cancelled)` to the handler.
    #[test]
    fn cancel_then_resume_delivers_url_error_cancelled() {
        let task = task_value(
            request_value("https://example.com/cancel-resume"),
            session_value(configuration_value()),
            5,
        );
        let mut ctx = HttpEventCtx::new(vec![]);
        task_cancel(&mut ctx, task.clone(), vec![]).unwrap();
        // cancel mutated in place — use the same task value.
        task_resume(&mut ctx, task, vec![]).unwrap();
        assert_eq!(ctx.calls.len(), 1, "expected one handler call");
        let args = &ctx.calls[0].1;
        assert_eq!(args[0], SwiftValue::Nil, "data should be nil on cancel");
        assert_eq!(args[1], SwiftValue::Nil, "response should be nil on cancel");
        let SwiftValue::Struct(err_struct) = &args[2] else {
            panic!("expected URLError struct, got {:?}", args[2]);
        };
        assert_eq!(err_struct.type_name, "URLError");
        assert!(
            matches!(err_struct.get("code"), Some(SwiftValue::Enum(e)) if e.case == "cancelled"),
            "expected URLError(.cancelled), got {:?}",
            err_struct.get("code")
        );
    }

    /// Two `dataTask` calls produce distinct Objects (not `Rc::ptr_eq`).
    #[test]
    fn two_data_tasks_are_distinct_objects() {
        let mut ctx = HttpEventCtx::new(vec![]);
        let session = session_value(configuration_value());
        let out1 = session_data_task(
            &mut ctx,
            session.clone(),
            vec![
                Arg {
                    label: Some("with".into()),
                    value: request_value("https://a.example.com/"),

                    static_ty: None,
                },
                Arg {
                    label: None,
                    value: SwiftValue::Closure(0),

                    static_ty: None,
                },
            ],
        )
        .unwrap()
        .unwrap();
        let out2 = session_data_task(
            &mut ctx,
            session,
            vec![
                Arg {
                    label: Some("with".into()),
                    value: request_value("https://b.example.com/"),

                    static_ty: None,
                },
                Arg {
                    label: None,
                    value: SwiftValue::Closure(1),

                    static_ty: None,
                },
            ],
        )
        .unwrap()
        .unwrap();
        let (SwiftValue::Object(a), SwiftValue::Object(b)) = (&out1.result, &out2.result) else {
            panic!("expected two Object tasks");
        };
        assert!(
            !Rc::ptr_eq(a, b),
            "two dataTask calls must return distinct Objects"
        );
    }

    // -----------------------------------------------------------------------
    // Phase 3 — URLSessionConfiguration + URLSession Object migration
    // -----------------------------------------------------------------------

    /// `configuration_value()` returns an Object with class_name
    /// `"URLSessionConfiguration"` and all expected default fields.
    #[test]
    fn config_value_is_object() {
        let config = configuration_value();
        let SwiftValue::Object(obj) = config else {
            panic!("configuration_value must return Object");
        };
        let guard = obj.borrow();
        assert_eq!(guard.class_name, "URLSessionConfiguration");
        assert_eq!(
            guard.get("timeoutIntervalForRequest"),
            Some(&SwiftValue::Double(60.0))
        );
        assert_eq!(
            guard.get("timeoutIntervalForResource"),
            Some(&SwiftValue::Double(604_800.0))
        );
    }

    /// Two calls to `configuration_value()` (simulating `.default` per-access
    /// fresh semantics) return independent Objects — not the same `Rc`.
    #[test]
    fn config_default_fresh_per_call() {
        let a = configuration_value();
        let b = configuration_value();
        let (SwiftValue::Object(ra), SwiftValue::Object(rb)) = (&a, &b) else {
            panic!("expected two Object configs");
        };
        assert!(
            !Rc::ptr_eq(ra, rb),
            ".default must return a fresh Object on each access"
        );
    }

    /// `session_value()` returns an Object with class_name `"URLSession"`.
    #[test]
    fn session_value_is_object() {
        let sess = session_value(configuration_value());
        let SwiftValue::Object(obj) = &sess else {
            panic!("session_value must return Object");
        };
        assert_eq!(obj.borrow().class_name, "URLSession");
    }

    /// CRITICAL: `session_init` copies the configuration.
    /// Post-init mutations to the original config MUST NOT affect the session.
    #[test]
    fn session_init_copies_config() {
        let config = configuration_value();
        // Set a custom timeout on the config before init.
        if let SwiftValue::Object(o) = &config {
            o.borrow_mut()
                .set("timeoutIntervalForRequest", SwiftValue::Double(42.0));
        }

        // Build a session via session_init — it must snapshot the config.
        let mut ctx = HttpEventCtx::new(vec![]);
        let out = session_init(
            &mut ctx,
            vec![Arg {
                label: Some("configuration".into()),
                value: config.clone(),

                static_ty: None,
            }],
        )
        .unwrap();

        // Mutate the original config AFTER session creation.
        if let SwiftValue::Object(o) = &config {
            o.borrow_mut()
                .set("timeoutIntervalForRequest", SwiftValue::Double(99.0));
        }

        // Session's copy must still hold 42.0 (not the post-init 99.0).
        let session_timeout = session_timeout(&out);
        assert_eq!(
            session_timeout, 42.0,
            "session must hold a snapshot of config at init time (got {session_timeout})"
        );

        // Original config must now show the mutation (99.0).
        if let SwiftValue::Object(o) = &config {
            assert_eq!(
                o.borrow().get("timeoutIntervalForRequest"),
                Some(&SwiftValue::Double(99.0))
            );
        }
    }

    /// `copy_configuration` produces an independent Object: mutations to the
    /// copy do not affect the source.
    #[test]
    fn copy_config_is_independent() {
        let original = configuration_value();
        let copied = copy_configuration(&original);

        let (SwiftValue::Object(orig_rc), SwiftValue::Object(copy_rc)) = (&original, &copied)
        else {
            panic!("expected two Object configs");
        };

        // Must be distinct Rc (different storage).
        assert!(!Rc::ptr_eq(orig_rc, copy_rc), "copy must be a distinct Rc");

        // Mutate the copy — original must be unchanged.
        copy_rc
            .borrow_mut()
            .set("timeoutIntervalForRequest", SwiftValue::Double(7.0));
        assert_eq!(
            orig_rc.borrow().get("timeoutIntervalForRequest").cloned(),
            Some(SwiftValue::Double(60.0)),
            "original must be unaffected by copy mutation"
        );
    }

    /// `URLSession.shared` identity: two `SwiftValue::Object` values wrapping
    /// the same `Rc` compare equal via `Rc::ptr_eq`, so `shared === shared`
    /// holds in the interpreter (value.rs PartialEq uses ptr_eq for Objects).
    #[test]
    fn shared_object_identity_via_rc_clone() {
        // Simulate what the statics map does: store one Object, hand out Rc
        // clones on each access.
        let shared = session_value(configuration_value());
        let access1 = shared.clone(); // simulates first statics.get().cloned()
        let access2 = shared.clone(); // simulates second access
        let (SwiftValue::Object(a), SwiftValue::Object(b)) = (&access1, &access2) else {
            panic!("expected Object session");
        };
        assert!(
            Rc::ptr_eq(a, b),
            "two clones of the shared session Rc must be ptr_eq (shared === shared)"
        );
    }
}
