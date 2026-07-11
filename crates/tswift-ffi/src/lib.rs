//! C-ABI native embedding host for tswift.
//!
//! Exposes a small `extern "C"` surface fronted by two Swift façades
//! (`TSwiftCore`, `TSwiftUI`). The boundary is *serialized*: every value that
//! crosses the ABI is a JSON string the caller must release with
//! [`tswift_string_free`]. See `docs/plan/native-host.md` and CONTEXT.md.
//!
//! All `unsafe` for the FFI lives in this crate, preserving ADR-0001's
//! FFI-only-unsafe rule.

use std::ffi::{c_char, c_void, CStr, CString};

mod host;
mod http;
mod run;
mod swiftui;
mod util;

pub use host::{tswift_host_respond, TswiftHostFn};
pub use http::{
    tswift_http_event, tswift_http_respond, TswiftHttpCancelFn, TswiftHttpHandler,
    TswiftHttpStartFn,
};

/// The lifespan-owning VM handle handed to C as an opaque pointer — the native
/// analogue of QuickJS's `JSContext`. Owns the reclaimable interpreter bundle
/// (grown in later tasks: one-shot run state, then the SwiftUI render session).
/// Created with [`tswift_context_new`] and freed with [`tswift_context_free`].
pub struct Context {
    /// The live SwiftUI render session (T3), if one has been compiled. Owns its
    /// interpreter bundle and is reclaimed on recompile or `Context` free.
    swiftui: Option<swiftui::SwiftUiSession>,
    /// One-shot HTTP handler (existing path, verbatim). When present,
    /// each script `URLSession` call invokes the handler synchronously.
    http: Option<http::HostHttpHandler>,
    /// Streaming HTTP handler config (M6, additive). When present, takes
    /// priority over `http`; each request uses the event-driven
    /// `start/next_event/cancel` seam with host-pushed JSON events.
    stream_http: Option<http::StreamingHandlerConfig>,
    /// Registered host-native functions (Epic #246). Installed into the
    /// interpreter in both the one-shot run and SwiftUI compile paths, the same
    /// place the HTTP transport is wired. Owned by the context: dropped (and the
    /// retained handler boxes released) when the context is freed.
    host_fns: Vec<host::HostFnRegistration>,
    /// Host-service capabilities the embedding has *explicitly* declared via
    /// [`tswift_declare_host_service`]. A service is available iff the host
    /// declares its namespace — never inferred from registered function names.
    /// Threaded into `tswift_foundation::install_with` so host-backed framework
    /// APIs gate cleanly when their backing service was not declared.
    host_caps: tswift_core::Capabilities,
}

impl Context {
    fn new() -> Self {
        Context {
            swiftui: None,
            http: None,
            stream_http: None,
            host_fns: Vec::new(),
            host_caps: tswift_core::Capabilities::none(),
        }
    }
}

impl Default for Context {
    fn default() -> Self {
        Context::new()
    }
}

/// Allocate a new [`Context`] and hand ownership to the caller as a raw pointer.
///
/// The returned pointer must be released exactly once with
/// [`tswift_context_free`]; otherwise the `Context` leaks.
#[no_mangle]
pub extern "C" fn tswift_context_new() -> *mut Context {
    Box::into_raw(Box::new(Context::new()))
}

/// Free a [`Context`] previously returned by [`tswift_context_new`].
///
/// # Safety
/// `ctx` must be either null or a pointer returned by [`tswift_context_new`]
/// that has not already been freed. Passing any other pointer, or freeing the
/// same pointer twice, is undefined behaviour. Null is accepted and ignored.
#[no_mangle]
pub unsafe extern "C" fn tswift_context_free(ctx: *mut Context) {
    if ctx.is_null() {
        return;
    }
    drop(Box::from_raw(ctx));
}

/// Move an owned `String` onto the heap as a C string for the caller to release
/// with [`tswift_string_free`].
///
/// Our JSON never contains an interior NUL byte; on the impossible chance it
/// does, the string is replaced with an empty one rather than panicking.
pub(crate) fn into_json_ptr(value: String) -> *mut c_char {
    match CString::new(value) {
        Ok(c) => c.into_raw(),
        Err(_) => CString::new("")
            .expect("empty CString is always valid")
            .into_raw(),
    }
}

/// Free a string previously returned by any tswift entry point.
///
/// # Safety
/// `s` must be either null or a pointer returned by a tswift entry point (i.e.
/// produced by [`into_json_ptr`]) that has not already been freed. Passing any
/// other pointer, or freeing twice, is undefined behaviour. Null is accepted
/// and ignored.
#[no_mangle]
pub unsafe extern "C" fn tswift_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    drop(CString::from_raw(s));
}

/// Borrow a C string argument as `&str`, or `None` if it is null or not UTF-8.
///
/// # Safety
/// `s` must be either null or a valid, NUL-terminated C string pointer that
/// stays alive for the entire lifetime `'a` of the returned borrow (not merely
/// the duration of this call). Callers must not let the borrow outlive `s`.
pub(crate) unsafe fn borrow_str<'a>(s: *const c_char) -> Option<&'a str> {
    if s.is_null() {
        return None;
    }
    CStr::from_ptr(s).to_str().ok()
}

/// Build the serialized error JSON used when an argument is malformed.
fn arg_error_json(message: &str) -> String {
    format!(
        "{{\"ok\":false,\"backend\":\"ffi\",\"error\":\"{}\"}}",
        tswift_core::result_json::escape(message)
    )
}

/// Compile and run a Swift `source` string through `ctx`, returning owned result
/// JSON (mirrors `tswift-wasm`'s `runSwift`). Release the result with
/// [`tswift_string_free`].
///
/// The run is self-contained per call; `ctx` is required (forward-compatible
/// with later persistent run state) and must be non-null.
///
/// # Safety
/// `ctx` must be a live pointer from [`tswift_context_new`]. `source` must be
/// null or a valid NUL-terminated C string. The returned pointer is owned by
/// the caller and must be freed once with [`tswift_string_free`].
#[no_mangle]
pub unsafe extern "C" fn tswift_run(ctx: *mut Context, source: *const c_char) -> *mut c_char {
    let Some(ctx) = ctx.as_mut() else {
        return into_json_ptr(arg_error_json("null context"));
    };
    let Some(source) = borrow_str(source) else {
        return into_json_ptr(arg_error_json("source is null or not valid UTF-8"));
    };
    into_json_ptr(run::run_impl(
        source,
        ctx.http,
        ctx.stream_http,
        &ctx.host_fns,
        ctx.host_caps,
    ))
}

/// Register `handler` (with its opaque `userdata`) as the HTTP transport for
/// scripts run through `ctx` (`URLSession` support). Pass a null handler to
/// remove it. See `src/http.rs` and the header for the JSON contract; the
/// handler must call [`tswift_http_respond`] synchronously.
///
/// # Safety
/// `ctx` must be a live pointer from [`tswift_context_new`]. `handler` (when
/// non-null) must remain callable, and `userdata` valid, until the handler is
/// replaced/removed or the context is freed.
#[no_mangle]
pub unsafe extern "C" fn tswift_set_http_handler(
    ctx: *mut Context,
    handler: Option<TswiftHttpHandler>,
    userdata: *mut c_void,
) {
    let Some(ctx) = ctx.as_mut() else {
        return;
    };
    ctx.http = handler.map(|handler| http::HostHttpHandler { handler, userdata });
}

/// Register an event-driven streaming HTTP transport for scripts run through
/// `ctx`. Takes priority over the one-shot handler set by
/// [`tswift_set_http_handler`] when both are installed.
///
/// `start_fn` is called once per script request; it must initiate the request
/// and return quickly (fire-and-forget). The host then pushes events from any
/// thread via [`tswift_http_event`]. `cancel_fn` is called to abort an
/// in-flight request (e.g. on timeout or `Task.cancel()`). Pass a null
/// `start_fn` to remove the streaming handler.
///
/// See `docs/plan/native-host.md` and `src/http.rs` for the full contract.
///
/// # Safety
/// `ctx` must be a live pointer from [`tswift_context_new`]. `start_fn` and
/// `cancel_fn` (when non-null) must remain callable, and `userdata` valid,
/// until the handler is replaced/removed or the context is freed.
#[no_mangle]
pub unsafe extern "C" fn tswift_set_http_stream_handler(
    ctx: *mut Context,
    start_fn: Option<TswiftHttpStartFn>,
    cancel_fn: Option<TswiftHttpCancelFn>,
    userdata: *mut c_void,
) {
    let Some(ctx) = ctx.as_mut() else {
        return;
    };
    ctx.stream_http = match (start_fn, cancel_fn) {
        (Some(start_fn), Some(cancel_fn)) => Some(http::StreamingHandlerConfig {
            start_fn,
            cancel_fn,
            userdata,
        }),
        _ => None,
    };
}

/// Register a host-native function on `ctx`, callable from interpreted Swift by
/// the name in its signature. `signature_json` is the compact schema documented
/// in `crates/tswift-core/src/host_bridge.rs`
/// (`{"name":…,"params":[…],"returns":…,"throws":…}`). `callback` is invoked
/// synchronously when interpreted code calls the function; it receives the
/// function name, a JSON array of validated arguments, and an in-flight `call`
/// token it must answer with [`tswift_host_respond`] before returning.
/// `userdata` is passed through verbatim. Registering the same name replaces the
/// prior registration.
///
/// Returns owned result JSON: `{"ok":true,"name":"<fn>","error":null}` on
/// success, or `{"ok":false,"name":null,"error":"<why>"}` if the signature is
/// malformed or arguments are null. Release it with [`tswift_string_free`].
///
/// # Safety
/// `ctx` must be a live pointer from [`tswift_context_new`]. `signature_json`
/// must be null or a valid NUL-terminated C string. `callback` (when non-null)
/// must remain callable, and `userdata` valid, until the function is
/// removed/replaced or the context is freed. The returned pointer is owned by
/// the caller and must be freed once with [`tswift_string_free`].
#[no_mangle]
pub unsafe extern "C" fn tswift_register_host_fn(
    ctx: *mut Context,
    signature_json: *const c_char,
    callback: Option<TswiftHostFn>,
    userdata: *mut c_void,
) -> *mut c_char {
    let Some(ctx) = ctx.as_mut() else {
        return into_json_ptr(host_register_error("null context"));
    };
    let Some(signature_json) = borrow_str(signature_json) else {
        return into_json_ptr(host_register_error(
            "signature_json is null or not valid UTF-8",
        ));
    };
    let Some(callback) = callback else {
        return into_json_ptr(host_register_error("callback is null"));
    };
    match host::register(&mut ctx.host_fns, signature_json, callback, userdata) {
        Ok(name) => into_json_ptr(format!(
            "{{\"ok\":true,\"name\":\"{}\",\"error\":null}}",
            tswift_core::result_json::escape(&name)
        )),
        Err(message) => into_json_ptr(host_register_error(&message)),
    }
}

/// Remove the host-native function named `name` from `ctx` (a no-op if it was
/// never registered). After removal, interpreted code can no longer call it.
///
/// # Safety
/// `ctx` must be a live pointer from [`tswift_context_new`]. `name` must be
/// null or a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn tswift_remove_host_fn(ctx: *mut Context, name: *const c_char) {
    let Some(ctx) = ctx.as_mut() else {
        return;
    };
    let Some(name) = borrow_str(name) else {
        return;
    };
    host::remove(&mut ctx.host_fns, name);
}

/// Declare that the host backs the host-service identified by `namespace`
/// (e.g. `"tswift.defaults"`, `"tswift.fs"`, `"tswift.db"`), enabling the
/// framework APIs layered on that service for scripts run through `ctx`.
///
/// This is the *explicit whole-service* declaration a framework capability
/// gate consults: a service is available iff its namespace is declared here.
/// Capabilities are never inferred from the individual host functions a host
/// happens to register. Declaring the same namespace twice is idempotent.
///
/// Returns owned result JSON: `{"ok":true,"namespace":"<ns>","error":null}` on
/// success, or `{"ok":false,"namespace":null,"error":"<why>"}` if `namespace`
/// is null/invalid or unknown. Release it with [`tswift_string_free`].
///
/// # Safety
/// `ctx` must be a live pointer from [`tswift_context_new`]. `namespace` must
/// be null or a valid NUL-terminated C string. The returned pointer is owned
/// by the caller and must be freed once with [`tswift_string_free`].
#[no_mangle]
pub unsafe extern "C" fn tswift_declare_host_service(
    ctx: *mut Context,
    namespace: *const c_char,
) -> *mut c_char {
    let Some(ctx) = ctx.as_mut() else {
        return into_json_ptr(host_service_error("null context"));
    };
    let Some(namespace) = borrow_str(namespace) else {
        return into_json_ptr(host_service_error("namespace is null or not valid UTF-8"));
    };
    match tswift_core::HostService::for_namespace(namespace) {
        Some(service) => {
            ctx.host_caps = ctx.host_caps.with(service);
            into_json_ptr(format!(
                "{{\"ok\":true,\"namespace\":\"{}\",\"error\":null}}",
                tswift_core::result_json::escape(namespace)
            ))
        }
        None => into_json_ptr(host_service_error(&format!(
            "unknown host-service namespace: {namespace}"
        ))),
    }
}

/// Build the `{"ok":false,…}` result JSON for a failed host-service declaration.
fn host_service_error(message: &str) -> String {
    format!(
        "{{\"ok\":false,\"namespace\":null,\"error\":\"{}\"}}",
        tswift_core::result_json::escape(message)
    )
}

/// Build the `{"ok":false,…}` result JSON for a failed host-fn registration.
fn host_register_error(message: &str) -> String {
    format!(
        "{{\"ok\":false,\"name\":null,\"error\":\"{}\"}}",
        tswift_core::result_json::escape(message)
    )
}

/// Compile a SwiftUI program through `ctx`, render its root view, and start a
/// live render session (replacing any prior one). Returns owned UIIR JSON;
/// release it with [`tswift_string_free`].
///
/// # Safety
/// `ctx` must be a live pointer from [`tswift_context_new`]. `source` must be
/// null or a valid NUL-terminated C string. The returned pointer is owned by
/// the caller and must be freed once with [`tswift_string_free`].
#[no_mangle]
pub unsafe extern "C" fn tswift_swiftui_compile(
    ctx: *mut Context,
    source: *const c_char,
) -> *mut c_char {
    let Some(ctx) = ctx.as_mut() else {
        return into_json_ptr(swiftui::compile_error_json("null context"));
    };
    let Some(source) = borrow_str(source) else {
        return into_json_ptr(swiftui::compile_error_json(
            "source is null or not valid UTF-8",
        ));
    };
    into_json_ptr(swiftui::compile_with_transport(
        &mut ctx.swiftui,
        source,
        ctx.http,
        ctx.stream_http,
        &ctx.host_fns,
        ctx.host_caps,
    ))
}

/// Route a host event into `ctx`'s live render session and return an owned
/// patch-stream JSON; release it with [`tswift_string_free`]. `event_json` is
/// an object `{"id":string,"event":string,"value"?:scalar}`.
///
/// # Safety
/// `ctx` must be a live pointer from [`tswift_context_new`]. `event_json` must
/// be null or a valid NUL-terminated C string. The returned pointer is owned by
/// the caller and must be freed once with [`tswift_string_free`].
#[no_mangle]
pub unsafe extern "C" fn tswift_swiftui_dispatch(
    ctx: *mut Context,
    event_json: *const c_char,
) -> *mut c_char {
    let Some(ctx) = ctx.as_mut() else {
        return into_json_ptr(swiftui::dispatch_error_json("null context"));
    };
    let Some(event_json) = borrow_str(event_json) else {
        return into_json_ptr(swiftui::dispatch_error_json(
            "event_json is null or not valid UTF-8",
        ));
    };
    into_json_ptr(swiftui::dispatch(&mut ctx.swiftui, event_json))
}

/// Fire any pending `.task {}` closures on `ctx`'s live render session and
/// return an owned patch-stream JSON in the same envelope as
/// [`tswift_swiftui_dispatch`] (`{"ok":bool,"patches":[…]|null,"error":
/// string|null}`); release it with [`tswift_string_free`]. Call once after a
/// successful [`tswift_swiftui_compile`] to run appear-time async work and show
/// post-mount state. Safe to call with no `.task` modifiers present (returns an
/// empty patch list).
///
/// # Safety
/// `ctx` must be a live pointer from [`tswift_context_new`]. The returned
/// pointer is owned by the caller and must be freed once with
/// [`tswift_string_free`].
#[no_mangle]
pub unsafe extern "C" fn tswift_swiftui_run_mount_tasks(ctx: *mut Context) -> *mut c_char {
    let Some(ctx) = ctx.as_mut() else {
        return into_json_ptr(swiftui::dispatch_error_json("null context"));
    };
    into_json_ptr(swiftui::run_mount_tasks(&mut ctx.swiftui))
}

/// Lint a SwiftUI `source` and return frontend diagnostics as owned JSON
/// (`{"ok":bool,"diagnostics":[{"line","col","severity","message"}]}`), without
/// rendering or mutating any session — the editor's live error-feedback channel.
/// Stateless: takes no context. Release the result with [`tswift_string_free`].
///
/// # Safety
/// `source` must be null or a valid NUL-terminated C string. The returned
/// pointer is owned by the caller and must be freed once with
/// [`tswift_string_free`].
#[no_mangle]
pub unsafe extern "C" fn tswift_diagnostics(source: *const c_char) -> *mut c_char {
    let Some(source) = borrow_str(source) else {
        return into_json_ptr(
            "{\"ok\":false,\"diagnostics\":[{\"line\":1,\"col\":1,\"severity\":\"error\",\"message\":\"source is null or not valid UTF-8\"}]}"
                .to_string(),
        );
    };
    into_json_ptr(swiftui::diagnose(source))
}

// ── Module (multi-file) entry points ─────────────────────────────────────────────

/// Compile and run a multi-file Swift module through `ctx`, returning owned
/// result JSON (same envelope as [`tswift_run`]). Release with
/// [`tswift_string_free`].
///
/// `module_json` is a NUL-terminated JSON string:
/// `{"files":[{"path":"…","contents":"…"},…]}`. Files are analyzed together
/// as one compilation unit ([`Analysis::analyze_program`]); each diagnostic is
/// attributed to its true originating file and file-local line/col, not just
/// the first file. The existing `tswift_run` single-string entry point is
/// unchanged (additive).
///
/// # Safety
/// `ctx` must be a live pointer from [`tswift_context_new`]. `module_json`
/// must be null or a valid NUL-terminated C string. The returned pointer is
/// owned by the caller and must be freed once with [`tswift_string_free`].
#[no_mangle]
pub unsafe extern "C" fn tswift_run_module(
    ctx: *mut Context,
    module_json: *const c_char,
) -> *mut c_char {
    let Some(ctx) = ctx.as_mut() else {
        return into_json_ptr(arg_error_json("null context"));
    };
    let Some(module_json) = borrow_str(module_json) else {
        return into_json_ptr(arg_error_json("module_json is null or not valid UTF-8"));
    };
    into_json_ptr(run::run_module_impl(
        module_json,
        ctx.http,
        ctx.stream_http,
        &ctx.host_fns,
        ctx.host_caps,
    ))
}

/// Lint a multi-file Swift module and return owned diagnostics JSON (same
/// envelope as [`tswift_diagnostics`]). Stateless: takes no context. Release
/// with [`tswift_string_free`].
///
/// `module_json` is a NUL-terminated JSON string:
/// `{"files":[{"path":"…","contents":"…"},…]}`.
///
/// # Safety
/// `module_json` must be null or a valid NUL-terminated C string. The returned
/// pointer is owned by the caller and must be freed once with
/// [`tswift_string_free`].
#[no_mangle]
pub unsafe extern "C" fn tswift_diagnostics_module(module_json: *const c_char) -> *mut c_char {
    let Some(module_json) = borrow_str(module_json) else {
        return into_json_ptr(
            "{\"ok\":false,\"diagnostics\":[{\"line\":1,\"col\":1,\"severity\":\"error\",\"message\":\"module_json is null or not valid UTF-8\"}]}"
                .to_string(),
        );
    };
    into_json_ptr(swiftui::diagnose_module(module_json))
}

/// List every declaration symbol (name/kind/file/line/container/signature)
/// across a multi-file module, returning owned JSON. Stateless: takes no
/// context. Release with [`tswift_string_free`].
///
/// `module_json` is a NUL-terminated JSON string:
/// `{"files":[{"path":"…","contents":"…"},…]}`. Each file is analyzed
/// independently (`tswift_frontend::symbols::list_symbols`), so a syntax
/// error in one file doesn't block symbols from the others. Response shape:
/// `{"ok":bool,"symbols":[{"name","kind","file","line","container"?,
/// "signature"?},…],"error"?:string}` — `ok` is false only when
/// `module_json` itself fails to parse.
///
/// # Safety
/// `module_json` must be null or a valid NUL-terminated C string. The returned
/// pointer is owned by the caller and must be freed once with
/// [`tswift_string_free`].
#[no_mangle]
pub unsafe extern "C" fn tswift_list_symbols(module_json: *const c_char) -> *mut c_char {
    let Some(module_json) = borrow_str(module_json) else {
        return into_json_ptr(
            "{\"ok\":false,\"symbols\":[],\"error\":\"module_json is null or not valid UTF-8\"}"
                .to_string(),
        );
    };
    into_json_ptr(run::symbols_impl(module_json))
}

/// Compile a multi-file SwiftUI module through `ctx` and start a live render
/// session. Returns owned UIIR JSON (same envelope as
/// [`tswift_swiftui_compile`]); release with [`tswift_string_free`].
///
/// `module_json` is a NUL-terminated JSON string:
/// `{"files":[{"path":"…","contents":"…"},…]}`.
///
/// # Safety
/// `ctx` must be a live pointer from [`tswift_context_new`]. `module_json`
/// must be null or a valid NUL-terminated C string. The returned pointer is
/// owned by the caller and must be freed once with [`tswift_string_free`].
#[no_mangle]
pub unsafe extern "C" fn tswift_swiftui_compile_module(
    ctx: *mut Context,
    module_json: *const c_char,
) -> *mut c_char {
    let Some(ctx) = ctx.as_mut() else {
        return into_json_ptr(swiftui::compile_error_json("null context"));
    };
    let Some(module_json) = borrow_str(module_json) else {
        return into_json_ptr(swiftui::compile_error_json(
            "module_json is null or not valid UTF-8",
        ));
    };
    into_json_ptr(swiftui::compile_module_with_transport(
        &mut ctx.swiftui,
        module_json,
        ctx.http,
        ctx.stream_http,
        &ctx.host_fns,
        ctx.host_caps,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drift guard: each `extern "C"` entry point must keep the exact ABI
    /// signature declared in `include/tswift_ffi.h`. Renaming or changing the
    /// argument/return types of any symbol breaks this coercion at compile
    /// time, flagging that the header must be updated in lockstep.
    #[test]
    fn c_abi_signatures_match_header() {
        let _new: extern "C" fn() -> *mut Context = tswift_context_new;
        let _free: unsafe extern "C" fn(*mut Context) = tswift_context_free;
        let _run: unsafe extern "C" fn(*mut Context, *const c_char) -> *mut c_char = tswift_run;
        let _compile: unsafe extern "C" fn(*mut Context, *const c_char) -> *mut c_char =
            tswift_swiftui_compile;
        let _dispatch: unsafe extern "C" fn(*mut Context, *const c_char) -> *mut c_char =
            tswift_swiftui_dispatch;
        let _run_mount_tasks: unsafe extern "C" fn(*mut Context) -> *mut c_char =
            tswift_swiftui_run_mount_tasks;
        let _diagnostics: unsafe extern "C" fn(*const c_char) -> *mut c_char = tswift_diagnostics;
        let _string_free: unsafe extern "C" fn(*mut c_char) = tswift_string_free;
        let _set_http: unsafe extern "C" fn(*mut Context, Option<TswiftHttpHandler>, *mut c_void) =
            tswift_set_http_handler;
        let _respond: unsafe extern "C" fn(*mut c_void, *const c_char) = tswift_http_respond;
        // M6 streaming symbols
        let _set_stream_http: unsafe extern "C" fn(
            *mut Context,
            Option<TswiftHttpStartFn>,
            Option<TswiftHttpCancelFn>,
            *mut c_void,
        ) = tswift_set_http_stream_handler;
        let _http_event: unsafe extern "C" fn(*mut c_void, *const c_char) = tswift_http_event;
        // Module (multi-file) entry points
        let _run_module: unsafe extern "C" fn(*mut Context, *const c_char) -> *mut c_char =
            tswift_run_module;
        let _diagnostics_module: unsafe extern "C" fn(*const c_char) -> *mut c_char =
            tswift_diagnostics_module;
        let _compile_module: unsafe extern "C" fn(*mut Context, *const c_char) -> *mut c_char =
            tswift_swiftui_compile_module;
        // Symbol listing (Slice 12/13)
        let _list_symbols: unsafe extern "C" fn(*const c_char) -> *mut c_char = tswift_list_symbols;
        // Host-native function registration (Epic #246)
        let _register_host_fn: unsafe extern "C" fn(
            *mut Context,
            *const c_char,
            Option<TswiftHostFn>,
            *mut c_void,
        ) -> *mut c_char = tswift_register_host_fn;
        let _remove_host_fn: unsafe extern "C" fn(*mut Context, *const c_char) =
            tswift_remove_host_fn;
        let _host_respond: unsafe extern "C" fn(*mut c_void, *const c_char) = tswift_host_respond;
        // Explicit host-service capability declaration (slice 1).
        let _declare_host_service: unsafe extern "C" fn(
            *mut Context,
            *const c_char,
        ) -> *mut c_char = tswift_declare_host_service;
    }

    /// Declaring a known namespace succeeds and turns on that service's
    /// capability; an unknown namespace is a structured error, not a panic.
    #[test]
    fn declare_host_service_toggles_capability() {
        let ctx = tswift_context_new();
        // Default: nothing declared.
        assert_eq!(
            unsafe { &*ctx }.host_caps,
            tswift_core::Capabilities::none()
        );

        let ns = CString::new("tswift.defaults").unwrap();
        let reg = unsafe { tswift_declare_host_service(ctx, ns.as_ptr()) };
        let json = unsafe { CStr::from_ptr(reg) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(reg) };
        assert!(json.contains("\"ok\":true"), "{json}");
        assert!(unsafe { &*ctx }
            .host_caps
            .contains(tswift_core::HostService::Defaults));
        assert!(!unsafe { &*ctx }
            .host_caps
            .contains(tswift_core::HostService::FileSystem));

        let bad = CString::new("tswift.bogus").unwrap();
        let reg = unsafe { tswift_declare_host_service(ctx, bad.as_ptr()) };
        let json = unsafe { CStr::from_ptr(reg) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(reg) };
        assert!(json.contains("\"ok\":false"), "{json}");
        assert!(json.contains("unknown host-service namespace"), "{json}");

        unsafe { tswift_context_free(ctx) };
    }

    #[test]
    fn declare_host_service_null_context_is_error() {
        let ns = CString::new("tswift.defaults").unwrap();
        let ptr = unsafe { tswift_declare_host_service(std::ptr::null_mut(), ns.as_ptr()) };
        let json = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(ptr) };
        assert!(json.contains("null context"), "{json}");
    }

    /// A multi-file program run through `tswift_run_module` resolves cross-file
    /// references (a type in one file used from `main.swift`).
    #[test]
    fn run_module_resolves_cross_file() {
        let ctx = tswift_context_new();
        let module = CString::new(
            r#"{"files":[
                {"path":"models.swift","contents":"struct P { let x: Int }\n"},
                {"path":"main.swift","contents":"let p = P(x: 5)\nprint(p.x)\n"}
            ]}"#,
        )
        .unwrap();
        let out = unsafe { tswift_run_module(ctx, module.as_ptr()) };
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(out) };
        unsafe { tswift_context_free(ctx) };
        assert!(json.contains("\"ok\":true"), "unexpected result: {json}");
        assert!(json.contains("5"), "unexpected stdout: {json}");
    }

    /// A compile error in the second file reports that file's path and its
    /// file-local line number through the module entry point.
    #[test]
    fn run_module_diagnostic_carries_file_and_line() {
        let ctx = tswift_context_new();
        let module = CString::new(
            r#"{"files":[
                {"path":"a.swift","contents":"struct A {}\nstruct B {}\n"},
                {"path":"main.swift","contents":"let x = 1\n#error(\"boom\")\n"}
            ]}"#,
        )
        .unwrap();
        let out = unsafe { tswift_run_module(ctx, module.as_ptr()) };
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(out) };
        unsafe { tswift_context_free(ctx) };
        assert!(json.contains("\"ok\":false"), "expected failure: {json}");
        assert!(json.contains("main.swift:2:"), "expected file:line: {json}");
    }

    /// Top-level executable code outside `main.swift` is rejected with a
    /// diagnostic naming the offending file.
    #[test]
    fn run_module_rejects_top_level_outside_main() {
        let ctx = tswift_context_new();
        let module = CString::new(
            r#"{"files":[
                {"path":"helpers.swift","contents":"func f() {}\nprint(\"nope\")\n"},
                {"path":"main.swift","contents":"f()\n"}
            ]}"#,
        )
        .unwrap();
        let out = unsafe { tswift_run_module(ctx, module.as_ptr()) };
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(out) };
        unsafe { tswift_context_free(ctx) };
        assert!(json.contains("\"ok\":false"), "expected failure: {json}");
        assert!(json.contains("helpers.swift"), "expected file name: {json}");
    }

    /// `tswift_list_symbols` is stateless (no context) and lists declarations
    /// across every file in the module, each carrying its own file/line and
    /// (for nested members) container.
    #[test]
    fn list_symbols_lists_across_files() {
        let module = CString::new(
            r#"{"files":[
                {"path":"Models.swift","contents":"struct Point {\n    let x: Int\n}\n"},
                {"path":"main.swift","contents":"func run() {}\n"}
            ]}"#,
        )
        .unwrap();
        let out = unsafe { tswift_list_symbols(module.as_ptr()) };
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(out) };
        assert!(json.contains("\"ok\":true"), "unexpected result: {json}");
        assert!(
            json.contains("\"name\":\"Point\",\"kind\":\"struct\""),
            "{json}"
        );
        assert!(
            json.contains("\"container\":\"Point\""),
            "expected x's container: {json}"
        );
        assert!(
            json.contains("\"name\":\"run\",\"kind\":\"func\",\"file\":\"main.swift\""),
            "{json}"
        );
    }

    /// A malformed `module_json` is a structured `{"ok":false,...}` error, not
    /// a panic — mirrors `tswift_diagnostics_module`'s null/UTF-8 handling.
    #[test]
    fn list_symbols_malformed_json_is_structured_error() {
        let bad = CString::new("not json").unwrap();
        let out = unsafe { tswift_list_symbols(bad.as_ptr()) };
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(out) };
        assert!(json.contains("\"ok\":false"), "{json}");
        assert!(json.contains("\"symbols\":[]"), "{json}");
    }

    /// A null `module_json` pointer is a structured error, not a crash.
    #[test]
    fn list_symbols_null_json_is_structured_error() {
        let out = unsafe { tswift_list_symbols(std::ptr::null()) };
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(out) };
        assert!(json.contains("\"ok\":false"), "{json}");
    }

    /// A host function `hostDeviceName() -> String` registered through the FFI
    /// is callable from a one-shot `tswift_run` script.
    #[test]
    fn register_host_fn_callable_from_run() {
        unsafe extern "C" fn device_name(
            _userdata: *mut c_void,
            _name: *const c_char,
            _args_json: *const c_char,
            call: *mut c_void,
        ) {
            let reply = CString::new(r#""iPhone""#).unwrap();
            tswift_host_respond(call, reply.as_ptr());
        }
        let ctx = tswift_context_new();
        let sig = CString::new(r#"{"name":"hostDeviceName","returns":"String"}"#).unwrap();
        let reg = unsafe {
            tswift_register_host_fn(ctx, sig.as_ptr(), Some(device_name), std::ptr::null_mut())
        };
        let reg_json = unsafe { CStr::from_ptr(reg) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(reg) };
        assert!(reg_json.contains("\"ok\":true"), "{reg_json}");
        assert!(reg_json.contains("hostDeviceName"), "{reg_json}");

        let source = CString::new("print(hostDeviceName())").unwrap();
        let out = unsafe { tswift_run(ctx, source.as_ptr()) };
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(out) };
        unsafe { tswift_context_free(ctx) };
        assert!(json.contains("iPhone"), "unexpected result: {json}");
    }

    /// A host function that receives an argument and echoes a computed result,
    /// exercising the argument-encoding half of the trampoline through the FFI.
    #[test]
    fn register_host_fn_receives_args() {
        unsafe extern "C" fn haptic(
            _userdata: *mut c_void,
            _name: *const c_char,
            args_json: *const c_char,
            call: *mut c_void,
        ) {
            let args = CStr::from_ptr(args_json).to_str().unwrap();
            // Echo the received style string back so the script can print it.
            let tswift_core::json::Json::Array(items) = tswift_core::json::parse(args).unwrap()
            else {
                panic!("expected array");
            };
            let tswift_core::json::Json::Str(style) = &items[0] else {
                panic!("expected string");
            };
            let reply = CString::new(format!("{:?}", format!("did {style}"))).unwrap();
            tswift_host_respond(call, reply.as_ptr());
        }
        let ctx = tswift_context_new();
        let sig = CString::new(
            r#"{"name":"hostHaptic","params":[{"label":"style","type":"String"}],"returns":"String"}"#,
        )
        .unwrap();
        unsafe {
            tswift_register_host_fn(ctx, sig.as_ptr(), Some(haptic), std::ptr::null_mut());
        }
        let source = CString::new(r#"print(hostHaptic(style: "tap"))"#).unwrap();
        let out = unsafe { tswift_run(ctx, source.as_ptr()) };
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(out) };
        unsafe { tswift_context_free(ctx) };
        assert!(json.contains("did tap"), "unexpected result: {json}");
    }

    /// A registered host function is available in a SwiftUI preview session on
    /// the same context, not just one-shot runs.
    #[test]
    fn register_host_fn_callable_from_swiftui_compile() {
        unsafe extern "C" fn device_name(
            _userdata: *mut c_void,
            _name: *const c_char,
            _args_json: *const c_char,
            call: *mut c_void,
        ) {
            let reply = CString::new(r#""iPad""#).unwrap();
            tswift_host_respond(call, reply.as_ptr());
        }
        let ctx = tswift_context_new();
        let sig = CString::new(r#"{"name":"hostDeviceName","returns":"String"}"#).unwrap();
        unsafe {
            tswift_register_host_fn(ctx, sig.as_ptr(), Some(device_name), std::ptr::null_mut());
        }
        let source =
            CString::new("struct V: View { var body: some View { Text(hostDeviceName()) } }")
                .unwrap();
        let compiled = unsafe { tswift_swiftui_compile(ctx, source.as_ptr()) };
        let json = unsafe { CStr::from_ptr(compiled) }
            .to_str()
            .unwrap()
            .to_string();
        unsafe { tswift_string_free(compiled) };
        unsafe { tswift_context_free(ctx) };
        assert!(json.contains("\"ok\":true"), "{json}");
        assert!(json.contains("iPad"), "unexpected tree: {json}");
    }

    /// Removing a registered host function makes subsequent script calls fail.
    #[test]
    fn removed_host_fn_is_not_callable() {
        unsafe extern "C" fn device_name(
            _userdata: *mut c_void,
            _name: *const c_char,
            _args_json: *const c_char,
            call: *mut c_void,
        ) {
            let reply = CString::new(r#""iPhone""#).unwrap();
            tswift_host_respond(call, reply.as_ptr());
        }
        let ctx = tswift_context_new();
        let sig = CString::new(r#"{"name":"hostDeviceName","returns":"String"}"#).unwrap();
        unsafe {
            tswift_register_host_fn(ctx, sig.as_ptr(), Some(device_name), std::ptr::null_mut());
        }
        let name = CString::new("hostDeviceName").unwrap();
        unsafe { tswift_remove_host_fn(ctx, name.as_ptr()) };
        let source = CString::new("print(hostDeviceName())").unwrap();
        let out = unsafe { tswift_run(ctx, source.as_ptr()) };
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(out) };
        unsafe { tswift_context_free(ctx) };
        // The call is now an unresolved function — the run must not succeed.
        assert!(
            !json.contains("iPhone"),
            "should not call removed fn: {json}"
        );
    }

    /// Registering a malformed signature returns an error envelope, not a panic.
    #[test]
    fn register_host_fn_malformed_signature_errors() {
        unsafe extern "C" fn cb(
            _u: *mut c_void,
            _n: *const c_char,
            _a: *const c_char,
            _c: *mut c_void,
        ) {
        }
        let ctx = tswift_context_new();
        let sig = CString::new(r#"{"params":[]}"#).unwrap();
        let reg =
            unsafe { tswift_register_host_fn(ctx, sig.as_ptr(), Some(cb), std::ptr::null_mut()) };
        let json = unsafe { CStr::from_ptr(reg) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(reg) };
        unsafe { tswift_context_free(ctx) };
        assert!(json.contains("\"ok\":false"), "{json}");
    }

    /// Integration test: streaming handler drives a full URLSession request via
    /// `tswift_run`.  The pusher thread delivers events while `tswift_run`
    /// blocks; the script prints the status code and body.
    #[test]
    fn run_with_stream_http_handler_serves_urlsession_scripts() {
        use std::sync::{Arc, Mutex};

        /// Raw pointer wrapper that is `Send` for this test only.
        struct SendPtr(*mut c_void);
        unsafe impl Send for SendPtr {}

        // Shared store so the pusher thread can get the task token from start_fn.
        let token_store: Arc<Mutex<Option<SendPtr>>> = Arc::new(Mutex::new(None));
        let token_store_for_start = token_store.clone();
        let userdata = Box::into_raw(Box::new(token_store_for_start)) as *mut c_void;

        unsafe extern "C" fn stream_start(
            userdata: *mut c_void,
            _req: *const c_char,
            token: *mut c_void,
        ) {
            let store = &*(userdata as *mut Arc<Mutex<Option<SendPtr>>>);
            *store.lock().unwrap() = Some(SendPtr(token));
        }
        unsafe extern "C" fn stream_cancel(_: *mut c_void, _: *mut c_void) {}

        let ctx = tswift_context_new();
        unsafe {
            tswift_set_http_stream_handler(ctx, Some(stream_start), Some(stream_cancel), userdata);
        }

        // Push events from a background thread once the token is available.
        let token_store_for_pusher = token_store.clone();
        let pusher = std::thread::spawn(move || {
            loop {
                let mut guard = token_store_for_pusher.lock().unwrap();
                if let Some(SendPtr(token)) = guard.take() {
                    drop(guard);
                    unsafe {
                        let resp = b"{\"event\":\"response\",\"status\":200,\"headers\":[[\"Content-Type\",\"text/plain\"]]}\0";
                        tswift_http_event(token, resp.as_ptr().cast());
                        // "aGVsbG8=" is base64 for "hello"
                        let chunk = b"{\"event\":\"chunk\",\"bodyBase64\":\"aGVsbG8=\"}\0";
                        tswift_http_event(token, chunk.as_ptr().cast());
                        let done = b"{\"event\":\"done\"}\0";
                        tswift_http_event(token, done.as_ptr().cast());
                    }
                    break;
                }
                drop(guard);
                std::thread::yield_now();
            }
        });

        let source = CString::new(
            "import Foundation\n\
             let (data, resp) = try await URLSession.shared.data(from: URL(string: \"https://x.example/\")!)\n\
             print((resp as! HTTPURLResponse).statusCode)\n\
             print(String(data: data, encoding: .utf8) ?? \"nil\")\n",
        )
        .unwrap();
        let out = unsafe { tswift_run(ctx, source.as_ptr()) };
        pusher.join().unwrap();
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        unsafe {
            tswift_string_free(out);
            tswift_context_free(ctx);
            drop(Box::from_raw(userdata as *mut Arc<Mutex<Option<SendPtr>>>));
        }
        assert!(json.contains("200\\nhello"), "unexpected result: {json}");
    }

    #[test]
    fn run_with_http_handler_serves_urlsession_scripts() {
        unsafe extern "C" fn handler(
            _userdata: *mut c_void,
            _request_json: *const c_char,
            call: *mut c_void,
        ) {
            // "aGVsbG8=" is "hello".
            let response = CString::new(
                r#"{"status": 200, "headers": [["Content-Type", "text/plain"]], "bodyBase64": "aGVsbG8="}"#,
            )
            .unwrap();
            tswift_http_respond(call, response.as_ptr());
        }
        let ctx = tswift_context_new();
        unsafe { tswift_set_http_handler(ctx, Some(handler), std::ptr::null_mut()) };
        let source = CString::new(
            "import Foundation\n\
             let (data, resp) = try await URLSession.shared.data(from: URL(string: \"https://x.example/\")!)\n\
             print((resp as! HTTPURLResponse).statusCode)\n\
             print(String(data: data, encoding: .utf8) ?? \"nil\")\n",
        )
        .unwrap();
        let out = unsafe { tswift_run(ctx, source.as_ptr()) };
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(out) };
        unsafe { tswift_context_free(ctx) };
        assert!(json.contains("200\\nhello"), "unexpected result: {json}");
    }

    #[test]
    fn swiftui_task_with_http_handler_updates_preview() {
        // Regression: the SwiftUI session interpreter must receive the context's
        // URLSession transport, so a `.task { await URLSession... }` fetch
        // resolves and `run_mount_tasks` patches the preview. Before the fix the
        // session had no transport, the fetch errored, and the view stayed on
        // its initial "loading" state.
        unsafe extern "C" fn handler(
            _userdata: *mut c_void,
            _request_json: *const c_char,
            call: *mut c_void,
        ) {
            // "b2sh" is base64 for "ok!".
            let response = CString::new(
                r#"{"status": 200, "headers": [["Content-Type", "text/plain"]], "bodyBase64": "b2sh"}"#,
            )
            .unwrap();
            tswift_http_respond(call, response.as_ptr());
        }
        let ctx = tswift_context_new();
        unsafe { tswift_set_http_handler(ctx, Some(handler), std::ptr::null_mut()) };
        let source = CString::new(
            "struct V: View {\n\
            \x20   @State private var label = \"loading\"\n\
            \x20   func load() async {\n\
            \x20       if let (data, _) = try? await URLSession.shared.data(from: URL(string: \"https://x.example/\")!),\n\
            \x20          let s = String(data: data, encoding: .utf8) { label = s }\n\
            \x20   }\n\
            \x20   var body: some View { Text(label).task { await load() } }\n\
            }\n",
        )
        .unwrap();
        let compiled = unsafe { tswift_swiftui_compile(ctx, source.as_ptr()) };
        let compiled_json = unsafe { CStr::from_ptr(compiled) }
            .to_str()
            .unwrap()
            .to_string();
        unsafe { tswift_string_free(compiled) };
        assert!(
            compiled_json.contains("\"ok\":true") && compiled_json.contains("loading"),
            "initial tree should show loading: {compiled_json}"
        );
        let patches = unsafe { tswift_swiftui_run_mount_tasks(ctx) };
        let patches_json = unsafe { CStr::from_ptr(patches) }
            .to_str()
            .unwrap()
            .to_string();
        unsafe { tswift_string_free(patches) };
        unsafe { tswift_context_free(ctx) };
        assert!(
            patches_json.contains("ok!"),
            "task fetch should patch the preview with the fetched body: {patches_json}"
        );
    }

    #[test]
    fn context_new_returns_nonnull_and_frees() {
        let ctx = tswift_context_new();
        assert!(!ctx.is_null());
        unsafe { tswift_context_free(ctx) };
    }

    #[test]
    fn context_free_null_is_noop() {
        unsafe { tswift_context_free(std::ptr::null_mut()) };
    }

    #[test]
    fn string_free_null_is_noop() {
        unsafe { tswift_string_free(std::ptr::null_mut()) };
    }

    #[test]
    fn json_ptr_round_trips_then_frees() {
        let ptr = into_json_ptr("{\"ok\":true}".to_string());
        assert!(!ptr.is_null());
        let read = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert_eq!(read, "{\"ok\":true}");
        unsafe { tswift_string_free(ptr) };
    }

    // --- T2: tswift_run -----------------------------------------------------

    /// Call `tswift_run` and return the (owned-then-freed) JSON as a `String`.
    fn run(ctx: *mut Context, source: &str) -> String {
        let csource = CString::new(source).unwrap();
        let ptr = unsafe { tswift_run(ctx, csource.as_ptr()) };
        assert!(!ptr.is_null());
        let json = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(ptr) };
        json
    }

    #[test]
    fn run_prints_to_stdout() {
        let ctx = tswift_context_new();
        let json = run(ctx, "print(\"hi\")");
        assert!(json.contains("\"ok\":true"), "{json}");
        assert!(json.contains("\"backend\":\"ffi\""), "{json}");
        assert!(json.contains("\"stdout\":\"hi\\n\""), "{json}");
        unsafe { tswift_context_free(ctx) };
    }

    #[test]
    fn run_reports_compile_error() {
        let ctx = tswift_context_new();
        let json = run(ctx, "#error(\"boom\")");
        assert!(json.contains("\"ok\":false"), "{json}");
        assert!(json.contains("\"compile\":{\"ok\":false"), "{json}");
        assert!(json.contains("boom"), "{json}");
        unsafe { tswift_context_free(ctx) };
    }

    #[test]
    fn run_reuses_context_without_stale_state() {
        let ctx = tswift_context_new();
        let first = run(ctx, "print(\"one\")");
        assert!(first.contains("\"stdout\":\"one\\n\""), "{first}");
        let second = run(ctx, "print(\"two\")");
        assert!(second.contains("\"ok\":true"), "{second}");
        assert!(second.contains("\"stdout\":\"two\\n\""), "{second}");
        unsafe { tswift_context_free(ctx) };
    }

    #[test]
    fn run_null_context_is_error_json() {
        let csource = CString::new("print(1)").unwrap();
        let ptr = unsafe { tswift_run(std::ptr::null_mut(), csource.as_ptr()) };
        let json = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(ptr) };
        assert!(json.contains("null context"), "{json}");
    }

    // --- diagnostics --------------------------------------------------------

    /// Call `tswift_diagnostics` and return the (owned-then-freed) JSON.
    fn diagnostics(source: &str) -> String {
        let csource = CString::new(source).unwrap();
        let ptr = unsafe { tswift_diagnostics(csource.as_ptr()) };
        assert!(!ptr.is_null());
        let json = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(ptr) };
        json
    }

    #[test]
    fn diagnostics_clean_swiftui_source_is_ok_and_empty() {
        // A well-formed View must lint clean (the spliced prelude resolves
        // View/Text/VStack/Button, and its own lines are filtered out).
        let json =
            diagnostics("struct V: View {\n  var body: some View {\n    Text(\"hi\")\n  }\n}");
        assert!(json.contains("\"ok\":true"), "{json}");
        assert!(json.contains("\"diagnostics\":[]"), "{json}");
    }

    #[test]
    fn diagnostics_user_error_maps_line_back_to_source() {
        // `#error` on the user's first line must report line 1 (not the
        // prelude-offset program line) with error severity and its message.
        let json = diagnostics("#error(\"boom\")");
        assert!(json.contains("\"ok\":false"), "{json}");
        assert!(json.contains("\"line\":1"), "{json}");
        assert!(json.contains("\"severity\":\"error\""), "{json}");
        assert!(json.contains("boom"), "{json}");
    }

    #[test]
    fn diagnostics_null_source_is_structured_error() {
        let ptr = unsafe { tswift_diagnostics(std::ptr::null()) };
        let json = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(ptr) };
        assert!(json.contains("\"ok\":false"), "{json}");
        assert!(json.contains("\"severity\":\"error\""), "{json}");
    }

    // ── Module (multi-file) entry points ───────────────────────────────────────

    /// Call `tswift_run_module` and return the (owned-then-freed) JSON as a `String`.
    fn run_module(ctx: *mut Context, module_json: &str) -> String {
        let cjson = CString::new(module_json).unwrap();
        let ptr = unsafe { tswift_run_module(ctx, cjson.as_ptr()) };
        assert!(!ptr.is_null());
        let json = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(ptr) };
        json
    }

    /// Cross-file reference: file B declares `greet()`, file A calls it.
    /// Verifies that multi-file concatenation resolves symbols across files.
    #[test]
    fn run_module_cross_file_reference_works() {
        let ctx = tswift_context_new();
        let module_json = r#"{"files":[
            {"path":"helpers.swift","contents":"func greet() -> String { return \"hello from module\" }"},
            {"path":"main.swift","contents":"print(greet())"}
        ]}"#;
        let json = run_module(ctx, module_json);
        assert!(json.contains("\"ok\":true"), "{json}");
        assert!(
            json.contains("hello from module"),
            "cross-file call must see the function: {json}"
        );
        unsafe { tswift_context_free(ctx) };
    }

    #[test]
    fn run_module_single_file_matches_run() {
        let ctx = tswift_context_new();
        let module_json = r#"{"files":[{"path":"main.swift","contents":"print(42)"}]}"#;
        let json = run_module(ctx, module_json);
        assert!(json.contains("\"ok\":true"), "{json}");
        assert!(json.contains("42"), "{json}");
        unsafe { tswift_context_free(ctx) };
    }

    #[test]
    fn run_module_null_context_is_error_json() {
        let cjson = CString::new(r#"{"files":[]}"#).unwrap();
        let ptr = unsafe { tswift_run_module(std::ptr::null_mut(), cjson.as_ptr()) };
        let json = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(ptr) };
        assert!(json.contains("null context"), "{json}");
    }

    #[test]
    fn run_module_malformed_json_is_error() {
        let ctx = tswift_context_new();
        let json = run_module(ctx, "not json");
        assert!(json.contains("\"ok\":false"), "{json}");
        unsafe { tswift_context_free(ctx) };
    }

    /// Call `tswift_diagnostics_module` and return the (owned-then-freed) JSON.
    fn diagnostics_module(module_json: &str) -> String {
        let cjson = CString::new(module_json).unwrap();
        let ptr = unsafe { tswift_diagnostics_module(cjson.as_ptr()) };
        assert!(!ptr.is_null());
        let json = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(ptr) };
        json
    }

    #[test]
    fn diagnostics_module_clean_source_is_ok() {
        let module_json = r#"{"files":[{"path":"main.swift","contents":"struct V: View {\n  var body: some View { Text(\"hi\") }\n}"}]}"#;
        let json = diagnostics_module(module_json);
        assert!(json.contains("\"ok\":true"), "{json}");
    }

    #[test]
    fn diagnostics_module_null_json_is_structured_error() {
        let ptr = unsafe { tswift_diagnostics_module(std::ptr::null()) };
        let json = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(ptr) };
        assert!(json.contains("\"ok\":false"), "{json}");
        assert!(json.contains("\"severity\":\"error\""), "{json}");
    }

    #[test]
    fn swiftui_compile_module_works() {
        let ctx = tswift_context_new();
        let module_json = r#"{"files":[{"path":"main.swift","contents":"struct V: View {\n  var body: some View { Text(\"hi\") }\n}"}]}"#;
        let cjson = CString::new(module_json).unwrap();
        let ptr = unsafe { tswift_swiftui_compile_module(ctx, cjson.as_ptr()) };
        let json = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(ptr) };
        assert!(json.contains("\"ok\":true"), "{json}");
        assert!(json.contains("\"root\":\"V\""), "{json}");
        unsafe { tswift_context_free(ctx) };
    }

    #[test]
    fn swiftui_compile_module_null_context_is_error() {
        let cjson = CString::new(r#"{"files":[]}"#).unwrap();
        let ptr = unsafe { tswift_swiftui_compile_module(std::ptr::null_mut(), cjson.as_ptr()) };
        let json = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        unsafe { tswift_string_free(ptr) };
        assert!(json.contains("\"ok\":false"), "{json}");
        assert!(json.contains("null context"), "{json}");
    }
}
