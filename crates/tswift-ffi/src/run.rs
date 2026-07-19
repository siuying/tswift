//! `tswift_run` — the one-shot compile+run entry point behind `TSwiftCore`.
//!
//! Mirrors `tswift-wasm`'s `run_swift_impl`, with `backend:"ffi"` and without
//! the `Box::leak`: the `Analysis` is owned for the call and handed to the
//! interpreter through a lifetime-scoped `&'static` borrow that never escapes,
//! so nothing leaks across repeated calls on a long-lived `Context`.

use tswift_core::json::{self, Json};
use tswift_core::result_json::{self, CompileReport, RunReport};
use tswift_frontend::{Analysis, SourceFile};

use crate::util::{elapsed_ms, now_ms};

const BACKEND: &str = "ffi";

/// Compile and run `source`, returning the result JSON (string body, owned).
/// A registered host HTTP handler (one-shot or streaming) becomes the run's
/// `URLSession` transport. The streaming config takes priority when both are set.
pub(crate) fn run_impl(
    source: &str,
    http: Option<crate::http::HostHttpHandler>,
    stream_http: Option<crate::http::StreamingHandlerConfig>,
    host_fns: &[crate::host::HostFnRegistration],
    caps: tswift_core::Capabilities,
) -> String {
    run_impl_named(source, "main.swift", http, stream_http, host_fns, caps)
}

/// Like [`run_impl`] but with an explicit diagnostic `filename`.
fn run_impl_named(
    source: &str,
    filename: &str,
    http: Option<crate::http::HostHttpHandler>,
    stream_http: Option<crate::http::StreamingHandlerConfig>,
    host_fns: &[crate::host::HostFnRegistration],
    caps: tswift_core::Capabilities,
) -> String {
    let started = now_ms();

    let analysis = match Analysis::analyze(source, filename) {
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
    run_with_analysis(
        analysis,
        filename,
        started,
        false,
        http,
        stream_http,
        host_fns,
        caps,
    )
}

/// Compile and run an ordered `[SourceFile]` program via
/// [`Analysis::analyze_program`]. Diagnostics carry their per-file paths.
#[allow(clippy::too_many_arguments)]
fn run_program_impl_files(
    files: &[SourceFile],
    http: Option<crate::http::HostHttpHandler>,
    stream_http: Option<crate::http::StreamingHandlerConfig>,
    host_fns: &[crate::host::HostFnRegistration],
    caps: tswift_core::Capabilities,
) -> String {
    let started = now_ms();
    let filename = files
        .first()
        .map(|f| f.path.clone())
        .unwrap_or_else(|| "main.swift".to_string());
    let analysis = match Analysis::analyze_program(files) {
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
    run_with_analysis(
        analysis,
        &filename,
        started,
        true,
        http,
        stream_http,
        host_fns,
        caps,
    )
}

/// Shared tail of the run path: format diagnostics (per-file when
/// `include_file`), then compile-gate and evaluate. Single-source callers pass
/// `include_file = false` to keep the historical `line:col:` diagnostic shape.
#[allow(clippy::too_many_arguments)]
fn run_with_analysis(
    analysis: Analysis,
    filename: &str,
    started: f64,
    include_file: bool,
    http: Option<crate::http::HostHttpHandler>,
    stream_http: Option<crate::http::StreamingHandlerConfig>,
    host_fns: &[crate::host::HostFnRegistration],
    caps: tswift_core::Capabilities,
) -> String {
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

    let ast_preview = analysis.root().dump_json();
    let compile_elapsed = elapsed_ms(started);

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
    let mut interp = tswift_core::Interpreter::new(&mut stdout);
    tswift_std::install(&mut interp);
    // iOS/native embeddings declare host-service capabilities *explicitly* via
    // `tswift_declare_host_service` (namespace strings). A service is available
    // iff its namespace was declared — never inferred from registered fn names.
    tswift_foundation::install_with(&mut interp, caps);
    tswift_swiftdata::install(
        &mut interp,
        caps.contains(tswift_core::HostService::Database),
    );
    interp.set_filename(filename);
    if let Some(config) = stream_http {
        interp.set_http_transport(Box::new(crate::http::StreamingHostHttpHandler::from(
            config,
        )));
    } else if let Some(handler) = http {
        interp.set_http_transport(Box::new(handler));
    }
    crate::host::install(&mut interp, host_fns);

    // SAFETY: `interp.run` requires `&'static Analysis` (ADR-0003). `analysis`
    // is declared before `interp`, so it outlives `interp` and is dropped after
    // it; the `&'static` borrow never escapes this function (no reference is
    // stored or returned). This confines the lifetime fib to the ffi crate.
    let static_analysis: &'static Analysis =
        unsafe { std::mem::transmute::<&Analysis, &'static Analysis>(&analysis) };
    let run_result = interp.run(static_analysis);
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

// ── Module helpers ──────────────────────────────────────────────────────────

/// A parsed `{"files":[{"path":"…","contents":"…"},…]}` module payload.
pub(crate) struct Module {
    /// Ordered list of `(path, contents)` pairs.
    pub files: Vec<(String, String)>,
}

impl Module {
    /// Concatenate all file contents (separated by a single newline) and return
    /// `(merged_source, first_file_path)`. If the module is empty, returns an
    /// empty string with path `"main.swift"`.
    pub fn merge(&self) -> (String, &str) {
        let filename = self
            .files
            .first()
            .map(|(p, _)| p.as_str())
            .unwrap_or("main.swift");
        let source = self
            .files
            .iter()
            .map(|(_, c)| c.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        (source, filename)
    }

    /// Convert to the ordered `[SourceFile]` program-input model consumed by
    /// [`Analysis::analyze_program`].
    pub fn source_files(&self) -> Vec<SourceFile> {
        self.files
            .iter()
            .map(|(p, c)| SourceFile::new(p.clone(), c.clone()))
            .collect()
    }
}

/// Parse a `{"files":[{"path":"…","contents":"…"},…]}` JSON string.
pub(crate) fn parse_module(module_json: &str) -> Result<Module, String> {
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

/// Compile and run a multi-file module, returning the result JSON.
pub(crate) fn run_module_impl(
    module_json: &str,
    http: Option<crate::http::HostHttpHandler>,
    stream_http: Option<crate::http::StreamingHandlerConfig>,
    host_fns: &[crate::host::HostFnRegistration],
    caps: tswift_core::Capabilities,
) -> String {
    let module = match parse_module(module_json) {
        Ok(m) => m,
        Err(e) => {
            return result_json::result(
                BACKEND,
                CompileReport {
                    ok: false,
                    diagnostics: &e,
                    ast_preview: "",
                    elapsed_ms: 0,
                },
                None,
            );
        }
    };
    let files = module.source_files();
    run_program_impl_files(&files, http, stream_http, host_fns, caps)
}

/// Discover every `@Test` in a `{"files":[…]}` module and return descriptor
/// JSON, without running any test. Shape:
/// `{"ok":bool,"tests":[…],"error"?:string}` — `ok` is false only when
/// `module_json` itself fails to parse.
pub(crate) fn list_tests_impl(module_json: &str) -> String {
    match parse_module(module_json) {
        Ok(module) => {
            let tests = tswift_testing::list_tests(&module.source_files());
            tswift_testing::descriptors_to_json(&tests)
        }
        Err(e) => tswift_testing::error_json(&e),
    }
}

/// Run a `{"files":[…]}` module's `@Test`s under `options_json`
/// (`{"filter":…,"ids":[…]}`) and return the report JSON. A malformed module
/// is a structured error envelope, not a panic.
pub(crate) fn run_tests_impl(module_json: &str, options_json: &str) -> String {
    let module = match parse_module(module_json) {
        Ok(m) => m,
        Err(e) => return tswift_testing::error_json(&e),
    };
    let options = tswift_testing::parse_run_options(options_json);
    let report = tswift_testing::run_tests(&module.source_files(), &options);
    tswift_testing::report_to_json(&report)
}

/// List every declaration symbol (name/kind/file/line/container/signature)
/// across a `{"files":[{"path":"…","contents":"…"},…]}` module, as JSON.
/// Shape: `{"ok":bool,"symbols":[…],"error"?:string}` — `ok` is false only
/// when `module_json` itself fails to parse.
pub(crate) fn symbols_impl(module_json: &str) -> String {
    let module = match parse_module(module_json) {
        Ok(m) => m,
        Err(e) => {
            return format!(
                "{{\"ok\":false,\"symbols\":[],\"error\":{}}}",
                json::to_string(&Json::Str(e))
            );
        }
    };
    let symbols = tswift_frontend::symbols::list_symbols(&module.source_files());
    format!(
        "{{\"ok\":true,\"symbols\":{}}}",
        tswift_frontend::symbols::to_json(&symbols)
    )
}
