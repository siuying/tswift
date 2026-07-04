#![forbid(unsafe_code)]

use tswift_core::result_json::{self, escape, CompileReport, RunReport};
use tswift_core::Interpreter;
use tswift_frontend::{Analysis, Severity};
use wasm_bindgen::prelude::*;

// ── Batch-response decoder (wasm-independent, tested natively) ──────────────

/// Decode a `tswiftHttp` response JSON into a queue of [`HttpEvent`]s.
///
/// The hook may return either of two forms:
///
/// - **Batch** (M7): `{"events":[{…},…]}` — each array element is decoded via
///   [`tswift_core::http::decode_event_json`]; decoding stops at the first
///   terminal event.  A malformed element short-circuits the whole batch into
///   a single `Failed{badServerResponse}` queue.
/// - **Scalar** (legacy): any other valid JSON object is treated as a one-shot
///   response (`{"status":…}` / `{"error":…}`) and wrapped via
///   [`tswift_core::http::SingleShotEvents`]; existing hooks keep working
///   unchanged.
///
/// This function is `pub(crate)` and lives **outside** any `#[cfg(wasm32)]`
/// block so the native unit tests can exercise it directly.
pub(crate) fn decode_batch_response(
    response_json: &str,
) -> std::collections::VecDeque<tswift_core::http::HttpEvent> {
    use std::collections::VecDeque;
    use tswift_core::http::{
        decode_event_json, decode_response_json, HttpError, HttpEvent, SingleShotEvents,
    };
    use tswift_core::json::{self, Json};

    macro_rules! bad {
        ($msg:expr) => {{
            let mut q = VecDeque::new();
            q.push_back(HttpEvent::Failed {
                code: "badServerResponse".into(),
                message: $msg.to_string(),
            });
            return q;
        }};
    }

    let root = match json::parse(response_json) {
        Ok(r) => r,
        Err(_) => bad!("tswiftHttp returned invalid JSON"),
    };

    if let Some(Json::Array(events_arr)) = root.get("events") {
        // ── Batch form ────────────────────────────────────────────────────
        let mut queue = VecDeque::new();
        for event_val in events_arr {
            let event_json = json::to_string(event_val);
            match decode_event_json(&event_json) {
                Ok(event) => {
                    let terminal = event.is_terminal();
                    queue.push_back(event);
                    if terminal {
                        break; // stop at first terminal; ignore any trailing events
                    }
                }
                Err(HttpError::Failed { code, message }) => {
                    let mut q = VecDeque::new();
                    q.push_back(HttpEvent::Failed { code, message });
                    return q;
                }
                Err(_) => bad!("unexpected transport error in batch decode"),
            }
        }
        if queue.is_empty() {
            bad!("tswiftHttp batch response has no events");
        }
        // Guarantee a terminal event so the interpreter loop always terminates.
        if !queue.back().map(|e| e.is_terminal()).unwrap_or(false) {
            queue.push_back(HttpEvent::Done);
        }
        queue
    } else {
        // ── Scalar / legacy form ──────────────────────────────────────────
        let outcome = decode_response_json(response_json);
        let mut sse = SingleShotEvents::from_outcome(outcome);
        let mut queue = VecDeque::new();
        loop {
            let e = sse.next_event();
            let terminal = e.is_terminal();
            queue.push_back(e);
            if terminal {
                break;
            }
        }
        queue
    }
}

mod swiftui;

const BACKEND: &str = "wasm";

/// Compile and run a single Swift source string, returning a JSON result.
///
/// This is the wasm entry point. The heavy lifting lives in [`run_swift_impl`],
/// which is platform-independent and exercised by the native unit tests.
#[wasm_bindgen(js_name = runSwift)]
pub fn run_swift(source: &str) -> String {
    install_panic_hook();
    run_swift_impl(source)
}

/// Lint `source` through the frontend and return its diagnostics as JSON,
/// **without** running the program. This is the editor's live error-feedback
/// channel (debounced on keystrokes) — cheap, side-effect free, and the single
/// source of truth shared with the `runSwift` compile phase.
///
/// Shape: `{"ok":bool,"diagnostics":[{"line":u32,"col":u32,"message":string,
/// "severity":"error"|"warning"}]}`. `ok` is false iff any diagnostic is an
/// error (i.e. compilation would fail). A hard analyze failure (interior NUL)
/// surfaces as a single error diagnostic at 1:1.
#[wasm_bindgen(js_name = swiftDiagnostics)]
pub fn swift_diagnostics(source: &str) -> String {
    install_panic_hook();
    diagnose_impl(source)
}

fn diagnose_impl(source: &str) -> String {
    let analysis = match Analysis::analyze(source, "main.swift") {
        Ok(analysis) => analysis,
        Err(error) => {
            return diagnostics_json(false, &[diagnostic_json(1, 1, "error", &error.to_string())])
        }
    };

    let mut items = Vec::new();
    let mut had_error = false;
    for diagnostic in analysis.diagnostics() {
        let severity = match diagnostic.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        had_error |= diagnostic.is_error();
        items.push(diagnostic_json(
            diagnostic.line,
            diagnostic.col,
            severity,
            &diagnostic.message,
        ));
    }
    diagnostics_json(!had_error, &items)
}

fn diagnostic_json(line: u32, col: u32, severity: &str, message: &str) -> String {
    format!(
        "{{\"line\":{line},\"col\":{col},\"severity\":\"{severity}\",\"message\":\"{}\"}}",
        escape(message)
    )
}

fn diagnostics_json(ok: bool, items: &[String]) -> String {
    format!("{{\"ok\":{ok},\"diagnostics\":[{}]}}", items.join(","))
}

fn run_swift_impl(source: &str) -> String {
    let started = now_ms();

    let analysis = match Analysis::analyze(source, "main.swift") {
        Ok(analysis) => analysis,
        Err(error) => {
            return result_json::result(
                BACKEND,
                CompileReport {
                    ok: false,
                    diagnostics: &error.to_string(),
                    ast_preview: "",
                    elapsed_ms: elapsed_ms(started),
                },
                None,
            );
        }
    };

    let mut diagnostics = String::new();
    let mut had_error = false;
    for diagnostic in analysis.diagnostics() {
        let kind = if diagnostic.is_error() {
            "error"
        } else {
            "warning"
        };
        diagnostics.push_str(&format!(
            "{}:{}: {kind}: {}\n",
            diagnostic.line, diagnostic.col, diagnostic.message
        ));
        had_error |= diagnostic.is_error();
    }

    let ast_preview = analysis.root().dump_json();
    let compile_elapsed = elapsed_ms(started);

    // An error-severity diagnostic (e.g. `#error`, a type error) fails
    // compilation: report it as such and never enter the run phase.
    if had_error {
        return result_json::result(
            BACKEND,
            CompileReport {
                ok: false,
                diagnostics: &diagnostics,
                ast_preview: &ast_preview,
                elapsed_ms: compile_elapsed,
            },
            None,
        );
    }

    let run_started = now_ms();
    let analysis: &'static Analysis = Box::leak(Box::new(analysis));
    let mut stdout = Vec::new();
    let mut interp = Interpreter::new(&mut stdout);
    tswift_std::install(&mut interp);
    tswift_foundation::install(&mut interp);
    interp.set_filename("main.swift");
    platform::install_http_transport(&mut interp);

    let run_result = interp.run(analysis);
    let run_elapsed = elapsed_ms(run_started);
    let stdout = String::from_utf8_lossy(&stdout);

    let run_stderr = match &run_result {
        Ok(()) => String::new(),
        Err(error) => format!("error: {}", error),
    };
    result_json::result(
        BACKEND,
        CompileReport {
            ok: true,
            diagnostics: &diagnostics,
            ast_preview: &ast_preview,
            elapsed_ms: compile_elapsed,
        },
        Some(RunReport {
            ok: run_result.is_ok(),
            stdout: &stdout,
            stderr: &run_stderr,
            elapsed_ms: run_elapsed,
        }),
    )
}

// ── Platform shims ──────────────────────────────────────────────────────────
// On wasm we call into JS (`performance.now`, `console.error`); on native
// (used by the unit tests below) we fall back to std equivalents so the crate
// compiles and runs off-target.

#[cfg(target_arch = "wasm32")]
mod platform {
    use tswift_core::http::{decode_response_json, encode_request_json};
    use tswift_core::{
        HttpError, HttpEvent, HttpRequest, HttpResponse, HttpTaskHandle, HttpTransport,
    };
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = performance, js_name = now)]
        fn performance_now() -> f64;
        #[wasm_bindgen(js_namespace = console, js_name = error)]
        fn console_error(msg: &str);
    }

    // The `URLSession` host hook.  The embedding page/worker opts in by
    // defining a **synchronous** `globalThis.tswiftHttp(requestJson) ->
    // responseJson` (ADR-0005 — the interpreter is single-threaded).  On the
    // main thread that means a scripted/cached answer or sync XHR; a worker
    // can bridge to async `fetch` via `Atomics.wait` + `SharedArrayBuffer`.
    //
    // ## Response forms
    //
    // ### Scalar (legacy — still accepted)
    //
    //   Success: `{"status":200,"headers":[["K","V"]],"bodyBase64":"<b64>"}`
    //   Failure: `{"error":"timedOut","message":"…"}`
    //
    // ### Batch (M7 — enables delegates, progress, cancellation replay)
    //
    //   ```json
    //   {"events":[
    //     {"event":"response","status":200,"headers":[["Content-Type","text/plain"]]},
    //     {"event":"chunk","bodyBase64":"aGVsbG8="},
    //     {"event":"done"}
    //   ]}
    //   ```
    //
    //   Each element follows the event-stream wire format from ADR-0011
    //   (`response` / `chunk` / `done` / `error`).  The runtime decodes events
    //   in order, dispatching delegate callbacks between each one.  Progress
    //   (`task.progress.fractionCompleted`) updates per chunk.
    //
    // ## Degraded-tier semantics (wasm)
    //
    // The fetch completes eagerly inside `tswiftHttp` — the hook returns the
    // full batch before the runtime dispatches any events.  Delegates and
    // progress replay faithfully in event order.  **Cancellation stops
    // delivery only**: `task.cancel()` / `Task.cancel()` prevents further
    // events from being dispatched but cannot abort the already-completed
    // native fetch.  True streaming requires SharedArrayBuffer + Atomics.wait
    // or an option-C resumable run surface — both deferred.
    //
    // Absent hook → `null` → `URLSession` reports itself unavailable.
    // A thrown exception → error-JSON transport failure, not a wasm trap.
    #[wasm_bindgen(inline_js = r#"
        export function tswift_http_call(requestJson) {
            const hook = globalThis.tswiftHttp;
            if (typeof hook !== "function") return null;
            try {
                return String(hook(requestJson));
            } catch (e) {
                return JSON.stringify({
                    error: "cannotConnectToHost",
                    message: String(e),
                });
            }
        }
    "#)]
    extern "C" {
        fn tswift_http_call(request_json: &str) -> Option<String>;
    }

    /// The `globalThis.tswiftHttp`-backed transport.
    ///
    /// Owns a per-handle event queue so that `start` / `next_event` / `cancel`
    /// work without the thread-local default shim.  See the hook comment above
    /// for the two accepted response forms (scalar legacy and batch M7).
    struct JsHttpTransport {
        next_id: u64,
        pending: std::collections::HashMap<u64, std::collections::VecDeque<HttpEvent>>,
    }

    impl JsHttpTransport {
        fn new() -> Self {
            JsHttpTransport {
                next_id: 0,
                pending: std::collections::HashMap::new(),
            }
        }
    }

    impl HttpTransport for JsHttpTransport {
        /// One-shot scalar path — used by the default `perform`-based callers
        /// that do not drive the event loop (backward compat).
        fn perform(&mut self, req: &HttpRequest) -> Result<HttpResponse, HttpError> {
            match tswift_http_call(&encode_request_json(req)) {
                Some(response_json) => decode_response_json(&response_json),
                None => Err(HttpError::Unavailable),
            }
        }

        /// Start a request: calls `tswiftHttp` once, decodes the response (scalar
        /// or batch), and stores the resulting event queue keyed by handle.
        fn start(&mut self, req: &HttpRequest) -> Result<HttpTaskHandle, HttpError> {
            let response_json = match tswift_http_call(&encode_request_json(req)) {
                Some(json) => json,
                None => return Err(HttpError::Unavailable),
            };
            self.next_id += 1;
            let id = self.next_id;
            let queue = super::decode_batch_response(&response_json);
            self.pending.insert(id, queue);
            Ok(HttpTaskHandle(id))
        }

        /// Pop the next queued event.  Returns `Failed{badServerResponse}` if
        /// the handle is unknown or already exhausted.
        fn next_event(&mut self, h: HttpTaskHandle) -> HttpEvent {
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

        /// Cancel delivery: drop any pending events and replace with a single
        /// terminal `Failed{cancelled}` so the next `next_event` call honours
        /// the cancel contract.  Cannot abort the fetch that already completed
        /// inside `tswiftHttp` (wasm degraded tier).
        fn cancel(&mut self, h: HttpTaskHandle) {
            let mut q = std::collections::VecDeque::new();
            q.push_back(HttpEvent::Failed {
                code: "cancelled".into(),
                message: "request cancelled".into(),
            });
            self.pending.insert(h.0, q);
        }
    }

    pub(super) fn install_http_transport(interp: &mut tswift_core::Interpreter<'_>) {
        interp.set_http_transport(Box::new(JsHttpTransport::new()));
    }

    pub(super) fn now_ms() -> f64 {
        performance_now()
    }

    pub(super) fn report_panic(msg: &str) {
        console_error(msg);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod platform {
    /// Off-target (native unit tests) there is no JS host: `URLSession` stays
    /// unavailable, matching a page that defines no `tswiftHttp` hook.
    pub(super) fn install_http_transport(_interp: &mut tswift_core::Interpreter<'_>) {}

    pub(super) fn now_ms() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as f64)
            .unwrap_or(0.0)
    }

    pub(super) fn report_panic(msg: &str) {
        eprintln!("{msg}");
    }
}

fn now_ms() -> f64 {
    platform::now_ms()
}

/// Forward Rust panics to `console.error` so the browser shows a real message
/// instead of an opaque `RuntimeError: unreachable`.
pub(crate) fn install_panic_hook() {
    use std::sync::Once;
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            platform::report_panic(&format!("tswift-wasm panic: {info}"));
        }));
    });
}

fn elapsed_ms(started: f64) -> u64 {
    (now_ms() - started).max(0.0).round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use tswift_core::http::HttpEvent;

    // -----------------------------------------------------------------------
    // decode_batch_response — pure logic, wasm-independent
    // -----------------------------------------------------------------------

    /// Build a compact event-stream JSON array for test fixtures.
    fn batch(events_json: &str) -> String {
        format!("{{\"events\":[{events_json}]}}")
    }

    const RESPONSE_200: &str =
        r#"{"event":"response","status":200,"headers":[["Content-Type","text/plain"]]}"#;
    const CHUNK_HI: &str = r#"{"event":"chunk","bodyBase64":"aGk="}"#; // base64("hi")
    const DONE: &str = r#"{"event":"done"}"#;

    #[test]
    fn batch_response_chunk_done_yields_three_events() {
        let json = batch(&format!("{RESPONSE_200},{CHUNK_HI},{DONE}"));
        let mut q = decode_batch_response(&json);
        assert!(
            matches!(q.pop_front(), Some(HttpEvent::Response { status: 200, .. })),
            "expected Response(200)"
        );
        assert_eq!(q.pop_front(), Some(HttpEvent::Chunk(b"hi".to_vec())));
        assert_eq!(q.pop_front(), Some(HttpEvent::Done));
        assert!(q.is_empty(), "unexpected trailing events");
    }

    #[test]
    fn batch_response_done_no_chunk_yields_two_events() {
        let json = batch(&format!("{RESPONSE_200},{DONE}"));
        let mut q = decode_batch_response(&json);
        assert!(matches!(
            q.pop_front(),
            Some(HttpEvent::Response { status: 200, .. })
        ));
        assert_eq!(q.pop_front(), Some(HttpEvent::Done));
        assert!(q.is_empty());
    }

    #[test]
    fn batch_error_event_yields_failed() {
        let error_ev = r#"{"event":"error","code":"timedOut","message":"timeout"}"#;
        let json = batch(error_ev);
        let mut q = decode_batch_response(&json);
        let ev = q.pop_front();
        assert!(
            matches!(&ev, Some(HttpEvent::Failed { code, .. }) if code == "timedOut"),
            "expected Failed{{timedOut}}, got {ev:?}"
        );
        assert!(q.is_empty());
    }

    #[test]
    fn batch_multiple_chunks_all_delivered() {
        let c1 = r#"{"event":"chunk","bodyBase64":"YQ=="}"#; // "a"
        let c2 = r#"{"event":"chunk","bodyBase64":"Yg=="}"#; // "b"
        let json = batch(&format!("{RESPONSE_200},{c1},{c2},{DONE}"));
        let q = decode_batch_response(&json);
        assert_eq!(q.len(), 4, "expected Response+2×Chunk+Done, got {q:?}");
    }

    #[test]
    fn batch_stops_at_first_terminal_ignores_trailing_events() {
        // Events after the first terminal must be silently dropped.
        let extra_chunk = r#"{"event":"chunk","bodyBase64":"dHJhaWw="}"#;
        let json = batch(&format!("{RESPONSE_200},{DONE},{extra_chunk}"));
        let q = decode_batch_response(&json);
        // Should be exactly Response + Done, the extra chunk is ignored.
        assert_eq!(q.len(), 2, "trailing events not dropped: {q:?}");
        assert!(matches!(q.back(), Some(HttpEvent::Done)));
    }

    #[test]
    fn batch_malformed_event_yields_bad_server_response() {
        let bad_ev = r#"{"event":"unknown_kind"}"}"#; // unknown event type
        let json = batch(bad_ev);
        let mut q = decode_batch_response(&json);
        let ev = q.pop_front();
        assert!(
            matches!(&ev, Some(HttpEvent::Failed { code, .. }) if code == "badServerResponse"),
            "expected Failed{{badServerResponse}}, got {ev:?}"
        );
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn batch_empty_events_array_yields_bad_server_response() {
        let json = batch(""); // {"events":[]}
        let mut q = decode_batch_response(&json);
        assert!(
            matches!(&q.pop_front(), Some(HttpEvent::Failed { code, .. }) if code == "badServerResponse"),
            "empty batch should be badServerResponse"
        );
    }

    #[test]
    fn batch_no_terminal_event_gets_done_appended() {
        // A batch that ends with non-terminal events should get Done appended
        // so the interpreter loop always terminates.
        let json = batch(&format!("{RESPONSE_200},{CHUNK_HI}"));
        let q = decode_batch_response(&json);
        assert!(
            matches!(q.back(), Some(HttpEvent::Done)),
            "expected Done appended, got {q:?}"
        );
    }

    // Scalar legacy form still works.
    #[test]
    fn scalar_success_response_wraps_as_response_chunk_done() {
        let scalar =
            r#"{"status":200,"headers":[["Content-Type","text/plain"]],"bodyBase64":"aGk="}"#;
        let mut q = decode_batch_response(scalar);
        assert!(matches!(
            q.pop_front(),
            Some(HttpEvent::Response { status: 200, .. })
        ));
        assert_eq!(q.pop_front(), Some(HttpEvent::Chunk(b"hi".to_vec())));
        assert_eq!(q.pop_front(), Some(HttpEvent::Done));
        assert!(q.is_empty());
    }

    #[test]
    fn scalar_error_response_yields_failed() {
        let scalar = r#"{"error":"timedOut","message":"timeout"}"#;
        let mut q = decode_batch_response(scalar);
        assert!(
            matches!(&q.pop_front(), Some(HttpEvent::Failed { code, .. }) if code == "timedOut"),
            "expected Failed{{timedOut}}"
        );
        assert!(q.is_empty());
    }

    #[test]
    fn invalid_json_yields_bad_server_response() {
        let mut q = decode_batch_response("not json at all");
        assert!(
            matches!(&q.pop_front(), Some(HttpEvent::Failed { code, .. }) if code == "badServerResponse"),
            "invalid JSON should be badServerResponse"
        );
    }

    // Minimal JSON field extraction good enough for these assertions; avoids a
    // serde_json dependency in a cdylib crate.
    fn bool_field(json: &str, key: &str) -> Option<bool> {
        let needle = format!("\"{key}\":");
        let rest = &json[json.find(&needle)? + needle.len()..];
        if rest.starts_with("true") {
            Some(true)
        } else if rest.starts_with("false") {
            Some(false)
        } else {
            None
        }
    }

    #[test]
    fn hello_world_runs() {
        // Regression guard: Interpreter::new() must not panic seeding its RNG
        // (this is exactly what broke on wasm via SystemTime::now()).
        let json = run_swift_impl("let who = \"Swift\"\nprint(\"Hello \\(who)!\")");
        assert_eq!(bool_field(&json, "ok"), Some(true), "json={json}");
        assert!(json.contains("Hello Swift!\\n"), "json={json}");
        assert!(json.contains("\"backend\":\"wasm\""), "json={json}");
    }

    #[test]
    fn analyze_error_yields_null_run() {
        // The only hard `analyze()` error is an interior NUL byte; it must take
        // the early branch with `run: null`.
        let json = run_swift_impl("let x = 1\0");
        assert_eq!(bool_field(&json, "ok"), Some(false), "json={json}");
        assert!(json.contains("\"run\":null"), "json={json}");
    }

    #[test]
    fn syntax_error_reports_compile_failure() {
        // Parse/semantic errors are error-severity diagnostics: compilation
        // fails and the run phase is skipped entirely (`run: null`).
        let json = run_swift_impl("let = = =");
        assert_eq!(bool_field(&json, "ok"), Some(false), "json={json}");
        assert!(json.contains("\"compile\":{\"ok\":false"), "json={json}");
        assert!(json.contains("\"run\":null"), "json={json}");
    }

    #[test]
    fn pound_error_reports_compile_failure() {
        // `#error` is an error-severity diagnostic: compile fails, run skipped.
        let json = run_swift_impl("#error(\"boom\")");
        assert_eq!(bool_field(&json, "ok"), Some(false), "json={json}");
        assert!(json.contains("\"compile\":{\"ok\":false"), "json={json}");
        assert!(json.contains("\"run\":null"), "json={json}");
        assert!(json.contains("error: boom"), "json={json}");
    }

    #[test]
    fn pound_warning_compiles_and_runs() {
        // `#warning` is advisory: compile succeeds and the program runs.
        let json = run_swift_impl("#warning(\"note\")\nprint(\"ok\")");
        assert_eq!(bool_field(&json, "ok"), Some(true), "json={json}");
        assert!(json.contains("\"compile\":{\"ok\":true"), "json={json}");
        assert!(json.contains("warning: note"), "json={json}");
    }

    #[test]
    fn runtime_error_compiles_but_run_fails() {
        // Compiles fine, traps at runtime (forced fatalError / out-of-range).
        let json = run_swift_impl("let a = [1, 2, 3]\nprint(a[9])");
        assert_eq!(bool_field(&json, "ok"), Some(false), "json={json}");
        // compile.ok is the first "ok" field and must be true.
        assert!(json.contains("\"compile\":{\"ok\":true"), "json={json}");
        assert!(json.contains("\"run\":{\"ok\":false"), "json={json}");
    }

    #[test]
    fn runs_with_multibyte_output() {
        // End-to-end: emoji/CJK output must round-trip through the JSON builder
        // without panicking on truncation or escaping.
        let json = run_swift_impl("print(\"\u{1F600}\u{4F60}\u{597D}\")");
        assert_eq!(bool_field(&json, "ok"), Some(true), "json={json}");
    }

    #[test]
    fn output_is_json_escaped() {
        let json = run_swift_impl("print(\"tab\\tquote\\\"end\")");
        assert!(json.contains("tab\\tquote\\\"end"), "json={json}");
    }

    #[test]
    fn produces_ast_preview() {
        let json = run_swift_impl("let x = 1");
        assert!(json.contains("astPreview"), "json={json}");
        assert!(json.contains("source_file"), "json={json}");
    }

    #[test]
    fn diagnostics_clean_source_is_ok_and_empty() {
        let json = diagnose_impl("let x = 1\nprint(x)");
        assert_eq!(bool_field(&json, "ok"), Some(true), "json={json}");
        assert!(json.contains("\"diagnostics\":[]"), "json={json}");
    }

    #[test]
    fn diagnostics_error_reports_position_and_severity() {
        // `#error` is an error-severity diagnostic with a known message.
        let json = diagnose_impl("#error(\"boom\")");
        assert_eq!(bool_field(&json, "ok"), Some(false), "json={json}");
        assert!(json.contains("\"severity\":\"error\""), "json={json}");
        assert!(json.contains("\"message\":\"boom\""), "json={json}");
        assert!(json.contains("\"line\":1"), "json={json}");
    }

    #[test]
    fn diagnostics_warning_keeps_ok_true() {
        // `#warning` is advisory: it appears but `ok` stays true (compiles).
        let json = diagnose_impl("#warning(\"note\")\nprint(\"ok\")");
        assert_eq!(bool_field(&json, "ok"), Some(true), "json={json}");
        assert!(json.contains("\"severity\":\"warning\""), "json={json}");
        assert!(json.contains("\"message\":\"note\""), "json={json}");
    }

    #[test]
    fn diagnostics_does_not_run_the_program() {
        // A program that would trap at runtime still lints clean (no run phase).
        let json = diagnose_impl("let a = [1, 2, 3]\nprint(a[9])");
        assert_eq!(bool_field(&json, "ok"), Some(true), "json={json}");
        assert!(json.contains("\"diagnostics\":[]"), "json={json}");
    }

    #[test]
    fn diagnostics_analyze_error_is_single_diagnostic() {
        // Interior NUL is the one hard analyze failure: one error at 1:1.
        let json = diagnose_impl("let x = 1\0");
        assert_eq!(bool_field(&json, "ok"), Some(false), "json={json}");
        assert!(json.contains("\"line\":1"), "json={json}");
        assert!(json.contains("\"severity\":\"error\""), "json={json}");
    }

    #[test]
    fn diagnostics_message_is_json_escaped() {
        let json = diagnose_impl("#error(\"a\\\"b\")");
        // The embedded quote must be escaped so the envelope stays valid JSON.
        assert!(json.contains("a\\\"b"), "json={json}");
    }
}
