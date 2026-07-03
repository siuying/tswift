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

mod http;
mod run;
mod swiftui;
mod util;

pub use http::{tswift_http_respond, TswiftHttpHandler};

/// The lifespan-owning VM handle handed to C as an opaque pointer — the native
/// analogue of QuickJS's `JSContext`. Owns the reclaimable interpreter bundle
/// (grown in later tasks: one-shot run state, then the SwiftUI render session).
/// Created with [`tswift_context_new`] and freed with [`tswift_context_free`].
pub struct Context {
    /// The live SwiftUI render session (T3), if one has been compiled. Owns its
    /// interpreter bundle and is reclaimed on recompile or `Context` free.
    swiftui: Option<swiftui::SwiftUiSession>,
    /// The host-registered HTTP handler backing `URLSession` in scripts run
    /// through this context (see `src/http.rs`); `None` means no network.
    http: Option<http::HostHttpHandler>,
}

impl Context {
    fn new() -> Self {
        Context {
            swiftui: None,
            http: None,
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
    into_json_ptr(run::run_impl(source, ctx.http))
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
    into_json_ptr(swiftui::compile(&mut ctx.swiftui, source))
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
        let _diagnostics: unsafe extern "C" fn(*const c_char) -> *mut c_char = tswift_diagnostics;
        let _string_free: unsafe extern "C" fn(*mut c_char) = tswift_string_free;
        let _set_http: unsafe extern "C" fn(*mut Context, Option<TswiftHttpHandler>, *mut c_void) =
            tswift_set_http_handler;
        let _respond: unsafe extern "C" fn(*mut c_void, *const c_char) = tswift_http_respond;
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
}
