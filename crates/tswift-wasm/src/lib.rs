#![forbid(unsafe_code)]

use tswift_core::json::{self, Json};
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
/// block so the native unit tests can exercise it directly. On non-wasm,
/// non-test builds it has no caller, so allow it to be unused there.
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
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

pub(crate) mod analysis_cache;
mod swiftui;

const BACKEND: &str = "wasm";

// ── Module helpers ──────────────────────────────────────────────────────────

struct Module {
    files: Vec<(String, String)>,
}

impl Module {
    /// The entry filename for runtime diagnostics: the first file's path.
    fn entry_filename(&self) -> &str {
        self.files
            .first()
            .map(|(p, _)| p.as_str())
            .unwrap_or("main.swift")
    }

    /// Concatenate all file contents (single newline separated) with the entry
    /// filename. Retained for the SwiftUI compile path, which wraps the merged
    /// source before analysis.
    fn merge(&self) -> (String, &str) {
        let source = self
            .files
            .iter()
            .map(|(_, c)| c.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        (source, self.entry_filename())
    }

    /// Convert to the ordered `[SourceFile]` program-input model consumed by
    /// [`Analysis::analyze_program`].
    fn source_files(&self) -> Vec<tswift_frontend::SourceFile> {
        self.files
            .iter()
            .map(|(p, c)| tswift_frontend::SourceFile::new(p.clone(), c.clone()))
            .collect()
    }
}

fn parse_module(module_json: &str) -> Result<Module, String> {
    let root = json::parse(module_json).map_err(|e| format!("module JSON parse error: {e}"))?;
    let arr = match root.get("files") {
        Some(Json::Array(a)) => a.clone(),
        _ => return Err("module JSON must have a \"files\" array".to_string()),
    };
    let mut files = Vec::with_capacity(arr.len());
    for item in &arr {
        let path = match item.get("path") {
            Some(Json::Str(s)) => s.clone(),
            _ => return Err("each file entry must have a \"path\" string".to_string()),
        };
        let contents = match item.get("contents") {
            Some(Json::Str(s)) => s.clone(),
            _ => return Err("each file entry must have a \"contents\" string".to_string()),
        };
        files.push((path, contents));
    }
    Ok(Module { files })
}

/// Register a host-native function so that interpreted Swift can call it.
///
/// `signature_json` is the compact JSON schema accepted by the core bridge:
///
/// ```json
/// {"name": "greet", "params": [{"label": "name", "type": "String"}], "returns": "String"}
/// ```
///
/// Returns `{"ok":true}` on success or `{"ok":false,"error":"…"}` when the
/// schema is malformed.  Registered functions are wired into every subsequent
/// `runSwift` / `runSwiftModule` call; the embedding page must also define a
/// **synchronous** `globalThis.tswiftHost(name, argsJson)` hook that services
/// the calls (see `platform` docs below).
#[wasm_bindgen(js_name = registerHostFunction)]
pub fn register_host_function(signature_json: &str) -> String {
    install_panic_hook();
    platform::register_host_function_schema(signature_json)
}

/// Clear all host functions registered via [`register_host_function`].
///
/// Intended for test harnesses that need a clean slate between runs.
/// Production pages generally register once per page load and never clear.
#[wasm_bindgen(js_name = clearHostFunctions)]
pub fn clear_host_functions() {
    install_panic_hook();
    platform::clear_host_function_schemas();
}

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

/// Compile and run a multi-file Swift module, returning a JSON result.
///
/// `module_json` is `{"files":[{"path":"…","contents":"…"},…]}`. Files are
/// analyzed together as one compilation unit ([`Analysis::analyze_program`]);
/// each diagnostic is attributed to its true originating file and file-local
/// line/col, not just the first file. Additive — `runSwift` remains
/// unchanged.
#[wasm_bindgen(js_name = runSwiftModule)]
pub fn run_swift_module(module_json: &str) -> String {
    install_panic_hook();
    let module = match parse_module(module_json) {
        Ok(m) => m,
        Err(e) => {
            return result_json::result(
                BACKEND,
                tswift_core::result_json::CompileReport {
                    ok: false,
                    diagnostics: &e,
                    ast_preview: "",
                    elapsed_ms: 0,
                },
                None,
            );
        }
    };
    run_program_impl(&module.source_files())
}

/// List every declaration symbol (name/kind/file/line/container/signature)
/// across a set of files, as JSON.
///
/// `files_json` is `{"files":[{"path":"…","contents":"…"},…]}` — the same
/// wire shape [`run_swift_module`] takes. Each file is analyzed
/// independently (`tswift_frontend::symbols::list_symbols`): a syntax error
/// in one file doesn't block symbols from the others. Shape:
/// `{"ok":bool,"symbols":[{"name","kind","file","line","container"?,
/// "signature"?},…],"error"?:string}` — `ok` is false only when `files_json`
/// itself fails to parse (in which case `symbols` is `[]`).
#[wasm_bindgen(js_name = listSymbols)]
pub fn list_symbols(files_json: &str) -> String {
    install_panic_hook();
    list_symbols_impl(files_json)
}

fn list_symbols_impl(files_json: &str) -> String {
    let module = match parse_module(files_json) {
        Ok(m) => m,
        Err(e) => {
            return format!(
                "{{\"ok\":false,\"symbols\":[],\"error\":{}}}",
                json::to_string(&Json::Str(e))
            )
        }
    };
    let symbols = tswift_frontend::symbols::list_symbols(&module.source_files());
    format!(
        "{{\"ok\":true,\"symbols\":{}}}",
        tswift_frontend::symbols::to_json(&symbols)
    )
}

/// Discover every `@Test` in a multi-file module and return descriptor JSON,
/// **without** running any test — the web playground's "list tests" seam.
///
/// `files_json` is `{"files":[{"path":"…","contents":"…"},…]}` (the same wire
/// shape [`run_swift_module`] takes). Response:
/// `{"ok":bool,"tests":[{"id","displayName","suitePath","file","line","tags",
/// "caseCount","cases","skipped","skipReason","target"},…],"error"?:string,
/// "compileError"?:string}` — `ok` is false when `files_json` itself fails to
/// parse (`error`) *or* when the module compiles but fails analysis
/// (`compileError`; unlike a parse failure, that means the module *did*
/// parse, it just doesn't type-check/build).
#[wasm_bindgen(js_name = listTests)]
pub fn list_tests(files_json: &str) -> String {
    install_panic_hook();
    match parse_module(files_json) {
        Ok(module) => {
            tswift_testing::list_result_to_json(&tswift_testing::list_tests(&module.source_files()))
        }
        Err(e) => tswift_testing::error_json(&e),
    }
}

/// Run a multi-file module's `@Test`s and return the structured report as JSON.
///
/// `files_json` is `{"files":[{"path":"…","contents":"…"},…]}`. `options_json`
/// is `{"filter":"…","ids":["…",…]}` (both optional; an empty string or `null`
/// runs everything). Response:
/// `{"ok":bool,"passed","failed","skipped","issueCount","durationMs",
/// "compileError","tests":[…]}`. Analysis/compile errors surface in
/// `compileError` with `ok:false`; there is no wasm-only side effect (the
/// runner captures stdout to a sink).
#[wasm_bindgen(js_name = runTests)]
pub fn run_tests(files_json: &str, options_json: &str) -> String {
    install_panic_hook();
    let module = match parse_module(files_json) {
        Ok(m) => m,
        Err(e) => return tswift_testing::error_json(&e),
    };
    let options = tswift_testing::parse_run_options(options_json);
    let report = tswift_testing::run_tests(&module.source_files(), &options);
    tswift_testing::report_to_json(&report)
}

/// Lint a multi-file Swift module and return diagnostics JSON.
///
/// `module_json` is `{"files":[{"path":"…","contents":"…"},…]}`.
/// Additive — `swiftDiagnostics` remains unchanged.
#[wasm_bindgen(js_name = swiftDiagnosticsModule)]
pub fn swift_diagnostics_module(module_json: &str) -> String {
    install_panic_hook();
    let module = match parse_module(module_json) {
        Ok(m) => m,
        Err(e) => {
            return diagnostics_json(false, &[diagnostic_json(1, 1, "error", &e)]);
        }
    };
    diagnose_program_impl(&module.source_files())
}

/// Lint a multi-file program via [`Analysis::analyze_program`], emitting
/// diagnostics whose JSON carries the originating file path (additive `file`
/// field). Single-file `diagnose_impl` is unchanged.
fn diagnose_program_impl(files: &[tswift_frontend::SourceFile]) -> String {
    let analysis = match Analysis::analyze_program(files) {
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
        items.push(diagnostic_json_with_file(
            diagnostic.line,
            diagnostic.col,
            severity,
            &diagnostic.message,
            diagnostic.file.as_deref(),
        ));
    }
    diagnostics_json(!had_error, &items)
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

/// Like [`diagnostic_json`] but additively includes a `"file"` field when the
/// diagnostic's originating file is known (multi-file programs).
fn diagnostic_json_with_file(
    line: u32,
    col: u32,
    severity: &str,
    message: &str,
    file: Option<&str>,
) -> String {
    match file {
        Some(file) => format!(
            "{{\"file\":\"{}\",\"line\":{line},\"col\":{col},\"severity\":\"{severity}\",\"message\":\"{}\"}}",
            escape(file),
            escape(message)
        ),
        None => diagnostic_json(line, col, severity, message),
    }
}

fn diagnostics_json(ok: bool, items: &[String]) -> String {
    format!("{{\"ok\":{ok},\"diagnostics\":[{}]}}", items.join(","))
}

fn run_swift_impl(source: &str) -> String {
    run_swift_impl_named(source, "main.swift")
}

/// Format an analysis's diagnostics into the newline-joined `stderr`-style
/// text embedded in the compile report. When `include_file` is set (multi-file
/// programs), each line is prefixed with the diagnostic's originating file
/// (`path:line:col: kind: message`); single-source runs keep the historical
/// `line:col: kind: message` shape unchanged.
fn collect_diagnostics(analysis: &Analysis, include_file: bool) -> (String, bool) {
    let mut diagnostics = String::new();
    let mut had_error = false;
    for diagnostic in analysis.diagnostics() {
        let kind = if diagnostic.is_error() {
            "error"
        } else {
            "warning"
        };
        match (include_file, &diagnostic.file) {
            (true, Some(file)) => diagnostics.push_str(&format!(
                "{file}:{}:{}: {kind}: {}\n",
                diagnostic.line, diagnostic.col, diagnostic.message
            )),
            _ => diagnostics.push_str(&format!(
                "{}:{}: {kind}: {}\n",
                diagnostic.line, diagnostic.col, diagnostic.message
            )),
        }
        had_error |= diagnostic.is_error();
    }
    (diagnostics, had_error)
}

fn run_swift_impl_named(source: &str, filename: &str) -> String {
    let started = now_ms();

    let analysis = match analysis_cache::analyze_cached(source, filename) {
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
    run_from_analysis(analysis, filename, started, false)
}

/// Compile and run a multi-file program (ordered `[SourceFile]`) via
/// [`Analysis::analyze_program`], returning the same JSON envelope as
/// [`run_swift_impl_named`]. Diagnostics carry their per-file paths.
fn run_program_impl(files: &[tswift_frontend::SourceFile]) -> String {
    let started = now_ms();
    let analysis = match analysis_cache::analyze_program_cached(files) {
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
    let filename = files
        .first()
        .map(|f| f.path.clone())
        .unwrap_or_else(|| "main.swift".to_string());
    run_from_analysis(analysis, &filename, started, true)
}

fn run_from_analysis(
    analysis: std::rc::Rc<Analysis>,
    filename: &str,
    started: f64,
    include_file: bool,
) -> String {
    let (diagnostics, had_error) = collect_diagnostics(&analysis, include_file);

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
    let mut stdout = Vec::new();
    let mut interp = Interpreter::new(&mut stdout);
    tswift_std::install(&mut interp);
    // The default host-call handler MUST be installed before
    // `tswift_foundation::install_with`: `HostBridge::register` resolves a
    // `None` handler against the default handler *at registration time*
    // (`crates/tswift-core/src/host_bridge.rs`), not lazily per-call. Since
    // Foundation's `tswift.defaults.*`/`tswift.fs.*` registrations pass
    // `None` (they don't own a handler — the platform does), registering them
    // before a default handler exists silently fails the registration
    // (`is_host_fn` then reports `false`, degrading every UserDefaults/
    // FileManager call as "unavailable" even when the page declared the
    // service in `globalThis.tswiftHostServices`).
    platform::install_host_call_handler(&mut interp);
    tswift_foundation::install_with(&mut interp, platform::host_capabilities());
    tswift_swiftdata::install(
        &mut interp,
        platform::host_capabilities().contains(tswift_core::HostService::Database),
    );
    interp.set_filename(filename);
    platform::install_http_transport(&mut interp);
    platform::install_registered_host_fns(&mut interp);

    // Retain the cached `Rc` for the interpreter's lifetime instead of leaking
    // to `'static`: the warm-start cache can evict its own `Rc` independently,
    // and the AST is freed once this (dropped-at-return) interpreter releases
    // its clone. See `analysis_cache` + `Interpreter::run_retaining`.
    let run_result = interp.run_retaining(analysis);
    let run_elapsed = elapsed_ms(run_started);
    // Drop the interpreter before reading its output buffer: this runs any
    // registered finalizers (e.g. closing SwiftData database handles) and
    // releases the `&mut stdout` borrow.
    drop(interp);
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
pub(crate) mod platform {
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

    /// Declare host-service capabilities for this page from an **explicit**
    /// host declaration: the optional `globalThis.tswiftHostServices` array of
    /// namespace strings (e.g. `["tswift.defaults", "tswift.fs"]`). Each
    /// recognised namespace enables its whole service; a page that defines no
    /// such array (or an empty one) backs no host services, so host-backed
    /// framework APIs gate cleanly. Capabilities are never inferred from the
    /// mere presence of the `tswiftHost` call bridge — a page may register
    /// arbitrary host functions without promising any of these services.
    pub(super) fn host_capabilities() -> tswift_core::Capabilities {
        let declared = tswift_host_services();
        tswift_core::Capabilities::from_namespaces(declared.split(',').filter(|ns| !ns.is_empty()))
    }

    // ── Host-function bridge ─────────────────────────────────────────────────
    //
    // The embedding page opts in by defining a **synchronous**
    // `globalThis.tswiftHost(name, argsJson) -> resultJson` hook.  The hook
    // receives the registered function name and a JSON-encoded argument array,
    // and must return a JSON-encoded result value (or `{"$thrown":"…"}` to
    // raise a catchable Swift error).
    //
    // ## Degraded-tier rules (mirrors `tswiftHttp`, ADR-0005)
    //
    // - Absent hook   → `null`                → runtime error "not available".
    // - Thrown JS exception → `{"$hostError":"…"}` sentinel from the shim
    //                       → `Err(message)` from [`JsHostCallHandler`]
    //                       → interpreter runtime error (NOT a wasm trap).
    //
    // wasm is single-threaded, so a `thread_local!` Vec is the right storage
    // for the schemas registered before each run.

    thread_local! {
        static HOST_FN_SCHEMAS: std::cell::RefCell<Vec<String>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    #[wasm_bindgen(inline_js = r#"
        export function tswift_host_call(name, argsJson) {
            const hook = globalThis.tswiftHost;
            if (typeof hook !== "function") return null;
            try {
                const result = hook(name, argsJson);
                return result == null ? "null" : String(result);
            } catch (e) {
                return JSON.stringify({ "$hostError": String(e) });
            }
        }
    "#)]
    extern "C" {
        fn tswift_host_call(name: &str, args_json: &str) -> Option<String>;
    }

    // Explicit host-service declaration: the page lists the namespaces it backs
    // in `globalThis.tswiftHostServices`. We return them comma-joined (empty
    // when the array is absent/empty/not an array) and map each to a service
    // in Rust — an absent declaration grants no host-backed capabilities.
    #[wasm_bindgen(inline_js = r#"
        export function tswift_host_services() {
            const s = globalThis.tswiftHostServices;
            if (!Array.isArray(s)) return "";
            return s.filter((x) => typeof x === "string").join(",");
        }
    "#)]
    extern "C" {
        fn tswift_host_services() -> String;
    }

    /// The `globalThis.tswiftHost`-backed [`HostCallHandler`].
    ///
    /// Forwards every call to the JS hook, intercepting the `$hostError`
    /// sentinel that the shim injects on thrown exceptions so they surface as
    /// Rust `Err` strings (interpreter runtime errors) rather than wasm traps.
    ///
    /// [`HostCallHandler`]: tswift_core::HostCallHandler
    struct JsHostCallHandler;

    impl tswift_core::HostCallHandler for JsHostCallHandler {
        fn call(&self, name: &str, args_json: &str) -> Result<String, String> {
            use tswift_core::json::{self, Json};
            let raw = match tswift_host_call(name, args_json) {
                Some(s) => s,
                None => return Err("tswiftHost hook is not available".into()),
            };
            // Detect the error sentinel the shim writes on JS exceptions.
            // The `$hostError` key is dollar-prefixed (like `$thrown`) and is
            // reserved — no legitimate handler result will contain it.
            if let Ok(root) = json::parse(&raw) {
                if let Some(Json::Str(msg)) = root.get("$hostError") {
                    return Err(msg.clone());
                }
            }
            Ok(raw)
        }
    }

    /// Validate `signature_json` and, if valid, push it into the thread-local
    /// registry so that the next run picks it up.  Returns a JSON status.
    pub(super) fn register_host_function_schema(signature_json: &str) -> String {
        use tswift_core::result_json::escape;
        match tswift_core::HostSignature::from_json(signature_json) {
            Ok(_) => {
                HOST_FN_SCHEMAS.with(|schemas| {
                    schemas.borrow_mut().push(signature_json.to_string());
                });
                r#"{"ok":true}"#.to_string()
            }
            Err(e) => {
                let escaped = escape(&e);
                format!(r#"{{"ok":false,"error":"{escaped}"}}"#)
            }
        }
    }

    /// Clear all registered host-function schemas (for test harnesses).
    pub(super) fn clear_host_function_schemas() {
        HOST_FN_SCHEMAS.with(|schemas| schemas.borrow_mut().clear());
    }

    /// Install the JS-backed host-call handler on `interp` as its **default**
    /// handler. Unconditional (not gated on any schema being registered): both
    /// custom `registerHostFunction` calls *and* Foundation's
    /// `tswift.defaults.*`/`tswift.fs.*` registrations (which pass `None` for
    /// their handler) resolve against this default. Must run before
    /// `tswift_foundation::install_with` — see the call site's comment.
    pub(super) fn install_host_call_handler(interp: &mut tswift_core::Interpreter<'_>) {
        use std::sync::Arc;
        interp.set_host_call_handler(Arc::new(JsHostCallHandler));
    }

    /// Register every custom host-fn schema pushed via
    /// [`register_host_function_schema`]. Must run after
    /// [`install_host_call_handler`] (schemas resolve `None` against whatever
    /// default handler is installed at registration time).
    pub(super) fn install_registered_host_fns(interp: &mut tswift_core::Interpreter<'_>) {
        HOST_FN_SCHEMAS.with(|schemas| {
            for sig_json in schemas.borrow().iter() {
                // Schemas were validated at registration time; ignore any
                // residual error (should not occur in practice).
                let _ = interp.register_host_fn(sig_json, None);
            }
        });
    }

    pub(super) fn now_ms() -> f64 {
        performance_now()
    }

    pub(super) fn report_panic(msg: &str) {
        console_error(msg);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod platform {
    // ── Off-target (native / cargo test) ────────────────────────────────────
    // No JS host is available; HTTP and host functions are unavailable.
    // The host-function registry still works so tests can verify schema
    // validation and that absent-hook errors surface cleanly.

    /// Off-target (native unit tests) there is no JS host: `URLSession` stays
    /// unavailable, matching a page that defines no `tswiftHttp` hook.
    pub(super) fn install_http_transport(_interp: &mut tswift_core::Interpreter<'_>) {}

    /// Off-target there is no JS host, so no host services are backed — the
    /// same degraded tier as a page that declares no `tswiftHostServices`.
    pub(super) fn host_capabilities() -> tswift_core::Capabilities {
        tswift_core::Capabilities::none()
    }

    thread_local! {
        static HOST_FN_SCHEMAS: std::cell::RefCell<Vec<String>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    /// Validate `signature_json` and store it.  Returns a JSON status string.
    pub(super) fn register_host_function_schema(signature_json: &str) -> String {
        use tswift_core::result_json::escape;
        match tswift_core::HostSignature::from_json(signature_json) {
            Ok(_) => {
                HOST_FN_SCHEMAS.with(|schemas| {
                    schemas.borrow_mut().push(signature_json.to_string());
                });
                r#"{"ok":true}"#.to_string()
            }
            Err(e) => {
                let escaped = escape(&e);
                format!(r#"{{"ok":false,"error":"{escaped}"}}"#)
            }
        }
    }

    /// Clear all registered host-function schemas.
    pub(super) fn clear_host_function_schemas() {
        HOST_FN_SCHEMAS.with(|schemas| schemas.borrow_mut().clear());
    }

    /// On native there is no JS hook, so registered functions always fail with
    /// "tswiftHost hook is not available" when called.  This mirrors the wasm
    /// absent-hook degraded tier so the error path is exercisable in tests.
    /// Unconditional, matching wasm's `install_host_call_handler` — see that
    /// function's doc comment for why it must run before `install_with`.
    pub(super) fn install_host_call_handler(interp: &mut tswift_core::Interpreter<'_>) {
        use std::sync::Arc;
        interp.set_host_call_handler(Arc::new(NativeUnavailableHandler));
    }

    /// Register every custom host-fn schema pushed via
    /// [`register_host_function_schema`]. Must run after
    /// [`install_host_call_handler`].
    pub(super) fn install_registered_host_fns(interp: &mut tswift_core::Interpreter<'_>) {
        HOST_FN_SCHEMAS.with(|schemas| {
            for sig_json in schemas.borrow().iter() {
                let _ = interp.register_host_fn(sig_json, None);
            }
        });
    }

    /// Stub handler used on native builds: every call returns an error
    /// matching the wasm absent-hook message so tests can assert on it.
    struct NativeUnavailableHandler;
    impl tswift_core::HostCallHandler for NativeUnavailableHandler {
        fn call(&self, _name: &str, _args_json: &str) -> Result<String, String> {
            Err("tswiftHost hook is not available".into())
        }
    }

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

    /// Build a self-contained program of approximately `repeats` code units
    /// (~8 lines each). Every unit is independent (suffixed by index) so the
    /// frontend does real lex/parse/sema work proportional to program size.
    #[cfg(test)]
    fn bench_program(repeats: usize) -> String {
        use std::fmt::Write;
        let unit = r#"struct Vec2_N { var x: Double; var y: Double
  func add(_ o: Vec2_N) -> Vec2_N { Vec2_N(x: x + o.x, y: y + o.y) }
  var length: Double { (x * x + y * y).squareRoot() }
}
func fib_N(_ n: Int) -> Int { n < 2 ? n : fib_N(n-1) + fib_N(n-2) }
var acc_N = 0
for i in 0..<12 { acc_N += fib_N(i % 10) }
let v_N = Vec2_N(x: 3, y: 4).add(Vec2_N(x: 1, y: 1))
print("acc=\(acc_N) len=\(v_N.length)")
"#;
        let mut src = String::new();
        for i in 0..repeats {
            let _ = write!(src, "{}", unit.replace('N', &i.to_string()));
        }
        src
    }

    /// Median of a slice of samples (sorts a copy; the slice is small).
    #[cfg(test)]
    fn median(samples: &[f64]) -> f64 {
        let mut v = samples.to_vec();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mid = v.len() / 2;
        if v.len() % 2 == 0 {
            (v[mid - 1] + v[mid]) / 2.0
        } else {
            v[mid]
        }
    }

    /// Warm-start micro-benchmark backing ADR-0018's measurement table.
    /// `#[ignore]` so it never runs in presubmit/CI; invoke explicitly:
    /// `cargo test -p tswift-wasm --release bench_warm_start -- --ignored --nocapture`.
    ///
    /// Reports **medians** (not best-of-N) over `SAMPLES` runs for two program
    /// sizes — the exact two rows ADR-0018 records. `cold` re-analyzes a
    /// freshly-unique source each sample (cache miss); `warm` re-runs one
    /// byte-identical source (cache hit), so `cold - warm` isolates the elided
    /// lex/parse/sema cost. The interpreter runs fresh in both cases.
    #[test]
    #[ignore = "benchmark; run with --ignored --nocapture"]
    fn bench_warm_start() {
        use std::time::Instant;
        const SAMPLES: usize = 51; // odd → exact median element

        // (label, repeat count) chosen so the emitted programs land near the
        // ~160-line and ~600-line sizes ADR-0018 tabulates.
        for (label, repeats) in [("small", 18usize), ("large", 66usize)] {
            let src = bench_program(repeats);
            let lines = src.lines().count();

            // cold: each sample is a distinct source (forces a cache miss +
            // full re-analyze), measuring the un-cached path.
            let mut cold = Vec::with_capacity(SAMPLES);
            for i in 0..SAMPLES {
                let s = format!("{src}\n// unique-{i}");
                let t = Instant::now();
                let _ = run_swift_impl(&s);
                cold.push(t.elapsed().as_secs_f64() * 1000.0);
            }

            // warm: prime once, then re-run the SAME source (cache hit).
            let _ = run_swift_impl(&src);
            let mut warm = Vec::with_capacity(SAMPLES);
            for _ in 0..SAMPLES {
                let t = Instant::now();
                let _ = run_swift_impl(&src);
                warm.push(t.elapsed().as_secs_f64() * 1000.0);
            }

            let cold_med = median(&cold);
            let warm_med = median(&warm);
            let saved = cold_med - warm_med;
            let pct = saved / cold_med * 100.0;
            println!(
                "BENCH {label:<5} lines={lines:<4} cold={cold_med:.3}ms warm={warm_med:.3}ms saved={saved:.3}ms ({pct:.0}%)"
            );
        }
    }

    /// Startup-cost breakdown (Slice 18). `#[ignore]` like `bench_warm_start`;
    /// invoke explicitly:
    /// `cargo test -p tswift-wasm --release bench_startup_breakdown -- --ignored --nocapture`.
    ///
    /// Splits a *cold* run into its four phases so we can see where first-run
    /// wall time actually goes before deciding whether a pre-analyzed prelude
    /// snapshot is worth building:
    ///   (a) install   — `tswift_std/foundation/swiftdata/swiftui` `install*`
    ///                    (the `register_*` builtin-table construction).
    ///   (b) prelude   — analyzing the SwiftUI + `@Query` preludes ALONE
    ///                    (the ~420-line boilerplate every SwiftUI compile pays).
    ///   (c) analyze   — analyzing prelude+user program, minus (b) = user code.
    ///   (d) execute   — `Interpreter::run_retaining` (tree-walk).
    /// Medians over `SAMPLES`; each phase is timed on a freshly-unique source
    /// so nothing is cache-warm.
    #[test]
    #[ignore = "benchmark; run with --ignored --nocapture"]
    fn bench_startup_breakdown() {
        use std::time::Instant;
        use tswift_swiftui::PRELUDE;
        const SAMPLES: usize = 51;

        let query_prelude = tswift_swiftdata::QUERY_PRELUDE;
        let charts_prelude = tswift_charts::PRELUDE;
        let prelude = format!("{PRELUDE}\n{query_prelude}\n{charts_prelude}\n");
        let prelude_lines = prelude.lines().count();

        // (a) install: build a full interpreter + run every framework install,
        // discarding it. This is the per-run register_* table construction the
        // wasm run/compile paths pay on every call.
        let mut install = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let mut sink = std::io::sink();
            let t = Instant::now();
            let mut interp = Interpreter::new(&mut sink);
            tswift_std::install(&mut interp);
            tswift_foundation::install_with(&mut interp, tswift_core::Capabilities::all());
            tswift_swiftdata::install(&mut interp, true);
            tswift_swiftui::install(&mut interp);
            tswift_charts::install(&mut interp);
            install.push(t.elapsed().as_secs_f64() * 1000.0);
            drop(interp);
        }

        // (b) prelude analysis: analyze ONLY the prelude boilerplate.
        let mut prelude_a = Vec::with_capacity(SAMPLES);
        for i in 0..SAMPLES {
            let src = format!("{prelude}// u{i}\n");
            let t = Instant::now();
            let _ = tswift_frontend::Analysis::analyze(&src, "main.swift");
            prelude_a.push(t.elapsed().as_secs_f64() * 1000.0);
        }

        for (label, repeats) in [("small", 18usize), ("large", 66usize)] {
            let user = bench_program(repeats);

            // (c) full analyze (prelude + user), unique each sample.
            let mut full_a = Vec::with_capacity(SAMPLES);
            for i in 0..SAMPLES {
                let src = format!("{prelude}{user}\n// u{i}");
                let t = Instant::now();
                let _ = tswift_frontend::Analysis::analyze(&src, "main.swift");
                full_a.push(t.elapsed().as_secs_f64() * 1000.0);
            }

            // (d) execute: analyze once (user-only program, no SwiftUI body so
            // it just runs top-level), then time the tree-walk repeatedly.
            let mut exec = Vec::with_capacity(SAMPLES);
            for _ in 0..SAMPLES {
                let analysis = tswift_frontend::Analysis::analyze(&user, "main.swift").unwrap();
                let rc = std::rc::Rc::new(analysis);
                let mut sink = std::io::sink();
                let mut interp = Interpreter::new(&mut sink);
                tswift_std::install(&mut interp);
                let t = Instant::now();
                let _ = interp.run_retaining(rc);
                exec.push(t.elapsed().as_secs_f64() * 1000.0);
                drop(interp);
            }

            let a = median(&install);
            let b = median(&prelude_a);
            let full = median(&full_a);
            let user_only = (full - b).max(0.0);
            let d = median(&exec);
            let lines = format!("{prelude}{user}").lines().count();
            println!(
                "BREAKDOWN {label:<5} lines={lines:<4} (prelude={prelude_lines}) \
                 install={a:.3}ms prelude_analyze={b:.3}ms user_analyze={user_only:.3}ms \
                 full_analyze={full:.3}ms execute={d:.3}ms"
            );
        }
    }

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
    fn run_swift_module_resolves_cross_file() {
        let module = r#"{"files":[
            {"path":"models.swift","contents":"struct P { let x: Int }\n"},
            {"path":"main.swift","contents":"let p = P(x: 7)\nprint(p.x)\n"}
        ]}"#;
        let json = run_swift_module(module);
        assert_eq!(bool_field(&json, "ok"), Some(true), "json={json}");
        assert!(json.contains("\"stdout\":\"7\\n\""), "json={json}");
    }

    #[test]
    fn run_swift_module_top_level_outside_main_fails() {
        let module = r#"{"files":[
            {"path":"helpers.swift","contents":"func f() {}\nprint(\"nope\")\n"},
            {"path":"main.swift","contents":"f()\n"}
        ]}"#;
        let json = run_swift_module(module);
        assert_eq!(bool_field(&json, "ok"), Some(false), "json={json}");
        assert!(json.contains("helpers.swift"), "json={json}");
    }

    #[test]
    fn list_symbols_lists_across_files_with_container() {
        let module = r#"{"files":[
            {"path":"Models.swift","contents":"struct Point {\n    let x: Int\n}\n"},
            {"path":"main.swift","contents":"func run() {}\n"}
        ]}"#;
        let json = list_symbols(module);
        assert_eq!(bool_field(&json, "ok"), Some(true), "json={json}");
        assert!(
            json.contains("\"name\":\"Point\",\"kind\":\"struct\""),
            "json={json}"
        );
        assert!(json.contains("\"name\":\"x\",\"kind\":\"let\",\"file\":\"Models.swift\",\"line\":2,\"container\":\"Point\""), "json={json}");
        assert!(
            json.contains("\"name\":\"run\",\"kind\":\"func\",\"file\":\"main.swift\""),
            "json={json}"
        );
    }

    #[test]
    fn list_symbols_reports_malformed_module_json() {
        let json = list_symbols("not json");
        assert_eq!(bool_field(&json, "ok"), Some(false), "json={json}");
        assert!(json.contains("\"symbols\":[]"), "json={json}");
    }

    #[test]
    fn diagnostics_module_carries_file_and_local_line() {
        let module = r#"{"files":[
            {"path":"a.swift","contents":"struct A {}\nstruct B {}\n"},
            {"path":"main.swift","contents":"let x = 1\n#error(\"boom\")\n"}
        ]}"#;
        let json = swift_diagnostics_module(module);
        assert_eq!(bool_field(&json, "ok"), Some(false), "json={json}");
        assert!(json.contains("\"file\":\"main.swift\""), "json={json}");
        assert!(json.contains("\"line\":2"), "json={json}");
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

    // -----------------------------------------------------------------------
    // registerHostFunction / tswiftHost transport — native-exercisable tests
    // -----------------------------------------------------------------------
    //
    // On native, `install_host_call_handler` uses `NativeUnavailableHandler` which
    // always returns "tswiftHost hook is not available".  That lets us verify:
    //  (a) schema validation in `register_host_function`,
    //  (b) that the registered function is wired into the interpreter,
    //  (c) that the absent-hook error surfaces as a runtime error.

    /// Helper: register a schema, run `src`, then clear and return the result.
    fn with_host_fn(schema: &str, src: &str) -> String {
        // Register the schema.
        let reg = platform::register_host_function_schema(schema);
        assert!(
            reg.contains("\"ok\":true"),
            "registration failed: {reg} for schema={schema}"
        );
        let result = run_swift_impl(src);
        // Always clear so other tests start with a clean slate.
        platform::clear_host_function_schemas();
        result
    }

    #[test]
    fn register_host_function_valid_schema_returns_ok() {
        let result =
            platform::register_host_function_schema(r#"{"name":"ping","returns":"String"}"#);
        platform::clear_host_function_schemas();
        assert!(result.contains("\"ok\":true"), "result={result}");
    }

    #[test]
    fn register_host_function_missing_name_returns_error() {
        let result = platform::register_host_function_schema(r#"{"returns":"Void"}"#);
        assert!(result.contains("\"ok\":false"), "result={result}");
        assert!(result.contains("\"error\""), "result={result}");
    }

    #[test]
    fn register_host_function_invalid_type_returns_error() {
        let result = platform::register_host_function_schema(r#"{"name":"f","returns":"Banana"}"#);
        assert!(result.contains("\"ok\":false"), "result={result}");
    }

    #[test]
    fn register_host_function_bad_json_returns_error() {
        let result = platform::register_host_function_schema("not json");
        assert!(result.contains("\"ok\":false"), "result={result}");
    }

    #[test]
    fn registered_host_fn_absent_hook_is_runtime_error() {
        // On native the hook is always absent, so calling a registered function
        // must fail as a runtime error (not a compile error and not a panic).
        let json = with_host_fn(
            r#"{"name":"ping","returns":"String"}"#,
            "let r = ping()\nprint(r)",
        );
        assert_eq!(bool_field(&json, "ok"), Some(false), "json={json}");
        // Compile phase must succeed (the function is registered).
        assert!(json.contains("\"compile\":{\"ok\":true"), "json={json}");
        // Run phase must fail with the absent-hook message.
        assert!(json.contains("\"run\":{\"ok\":false"), "json={json}");
        assert!(
            json.contains("tswiftHost hook is not available"),
            "json={json}"
        );
    }

    #[test]
    fn registered_void_host_fn_absent_hook_is_runtime_error() {
        let json = with_host_fn(r#"{"name":"doThing","returns":"Void"}"#, "doThing()");
        assert_eq!(bool_field(&json, "ok"), Some(false), "json={json}");
        assert!(json.contains("\"compile\":{\"ok\":true"), "json={json}");
        assert!(
            json.contains("tswiftHost hook is not available"),
            "json={json}"
        );
    }

    #[test]
    fn registered_labelled_fn_absent_hook_is_runtime_error() {
        let json = with_host_fn(
            r#"{"name":"greet","params":[{"label":"name","type":"String"}],"returns":"String"}"#,
            r#"let r = greet(name: "Sam")\nprint(r)"#,
        );
        assert_eq!(bool_field(&json, "ok"), Some(false), "json={json}");
        assert!(json.contains("\"compile\":{\"ok\":true"), "json={json}");
        assert!(
            json.contains("tswiftHost hook is not available"),
            "json={json}"
        );
    }

    #[test]
    fn clear_host_functions_removes_registered_schemas() {
        // Register, clear, then run — the function is unknown at runtime
        // (the interpreter defers free-function dispatch to the run phase).
        let reg = platform::register_host_function_schema(r#"{"name":"ping","returns":"Void"}"#);
        assert!(reg.contains("\"ok\":true"), "reg={reg}");
        platform::clear_host_function_schemas();
        // After clearing, `ping` is not wired: the run phase fails with
        // "unknown function".
        let json = run_swift_impl("ping()");
        assert_eq!(bool_field(&json, "ok"), Some(false), "json={json}");
        assert!(json.contains("unknown function"), "json={json}");
    }
}
