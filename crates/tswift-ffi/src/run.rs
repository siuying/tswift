//! `tswift_run` — the one-shot compile+run entry point behind `TSwiftCore`.
//!
//! Mirrors `tswift-wasm`'s `run_swift_impl`, with `backend:"ffi"` and without
//! the `Box::leak`: the `Analysis` is owned for the call and handed to the
//! interpreter through a lifetime-scoped `&'static` borrow that never escapes,
//! so nothing leaks across repeated calls on a long-lived `Context`.

use tswift_core::result_json::{self, CompileReport, RunReport};
use tswift_frontend::Analysis;

use crate::util::{elapsed_ms, now_ms};

const BACKEND: &str = "ffi";

/// Compile and run `source`, returning the result JSON (string body, owned).
pub(crate) fn run_impl(source: &str) -> String {
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
    tswift_foundation::install(&mut interp);
    interp.set_filename("main.swift");

    // SAFETY: `interp.run` requires `&'static Analysis` (ADR-0003). `analysis`
    // is declared before `interp`, so it outlives `interp` and is dropped after
    // it; the `&'static` borrow never escapes this function (no reference is
    // stored or returned). This confines the lifetime fib to the ffi crate.
    let static_analysis: &'static Analysis =
        unsafe { std::mem::transmute::<&Analysis, &'static Analysis>(&analysis) };
    let run_result = interp.run(static_analysis);
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
