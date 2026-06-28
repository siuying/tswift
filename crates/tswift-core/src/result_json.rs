//! Shared shaping for the compile+run result JSON emitted by the host backends.
//!
//! Both the wasm host (`tswift-wasm`) and the native FFI host (`tswift-ffi`)
//! report a single run as the same `{ok, backend, compile, run}` envelope; the
//! only per-backend differences are the `backend` tag and each crate's own
//! `Analysis` lifetime strategy (wasm `Box::leak`, ffi a scoped `&'static`).
//! The pure string shaping — escaping, truncation, and the four envelope shapes
//! — lives here so the two backends stay byte-for-byte identical (modulo
//! `backend`). This module is `unsafe`-free, so `tswift-wasm` keeps its
//! `#![forbid(unsafe_code)]`.

/// Max bytes of `astPreview` retained before truncation.
pub const AST_PREVIEW_LIMIT: usize = 6_000;
/// Max bytes of captured `stdout` retained before truncation.
pub const STDOUT_LIMIT: usize = 24_000;

/// The compile phase of a run.
pub struct CompileReport<'a> {
    /// Whether compilation succeeded (no error-severity diagnostics).
    pub ok: bool,
    /// Diagnostics text, emitted as `compile.stderr` (escaped).
    pub diagnostics: &'a str,
    /// AST preview JSON, emitted as `compile.astPreview` (truncated + escaped).
    pub ast_preview: &'a str,
    /// Wall-clock milliseconds spent compiling.
    pub elapsed_ms: u64,
}

/// The run phase of a run (absent when compilation failed).
pub struct RunReport<'a> {
    /// Whether the program ran to completion without a runtime error.
    pub ok: bool,
    /// Captured standard output (truncated + escaped).
    pub stdout: &'a str,
    /// Runtime error text (e.g. `"error: …"`) or `""` on success (escaped).
    pub stderr: &'a str,
    /// Wall-clock milliseconds spent running.
    pub elapsed_ms: u64,
}

/// Build the `{ok, backend, compile, run}` result envelope. `run` is `None`
/// when compilation failed (emitting `"run":null`); the overall `ok` is true
/// only when compilation succeeded *and* the run succeeded.
pub fn result(backend: &str, compile: CompileReport<'_>, run: Option<RunReport<'_>>) -> String {
    let overall_ok = compile.ok && run.as_ref().is_some_and(|r| r.ok);
    let run_json = match &run {
        None => "null".to_string(),
        Some(r) => format!(
            "{{\"ok\":{},\"stdout\":\"{}\",\"stderr\":\"{}\",\"elapsedMs\":{}}}",
            r.ok,
            escape(&truncate(r.stdout, STDOUT_LIMIT)),
            escape(r.stderr),
            r.elapsed_ms
        ),
    };
    format!(
        "{{\"ok\":{},\"backend\":\"{}\",\"compile\":{{\"ok\":{},\"stderr\":\"{}\",\"astPreview\":\"{}\",\"elapsedMs\":{}}},\"run\":{}}}",
        overall_ok,
        escape(backend),
        compile.ok,
        escape(compile.diagnostics),
        escape(&truncate(compile.ast_preview, AST_PREVIEW_LIMIT)),
        compile.elapsed_ms,
        run_json
    )
}

/// Escape `value` as the contents of a JSON string (no surrounding quotes),
/// handling quotes, backslashes, and all control characters (`\u00XX`).
pub fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => escaped.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => escaped.push(ch),
        }
    }
    escaped
}

/// Truncate `value` to at most `max` bytes, appending a marker noting the number
/// of dropped bytes. Slices on a UTF-8 char boundary at or before `max` so a
/// multibyte character straddling the limit never panics.
pub fn truncate(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let mut end = max;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n\n[output truncated {} bytes]",
        &value[..end],
        value.len() - end
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_handles_quotes_newlines_and_controls() {
        assert_eq!(escape("a\"b\\c\n\t\r"), "a\\\"b\\\\c\\n\\t\\r");
        assert_eq!(escape("\u{0007}"), "\\u0007");
    }

    #[test]
    fn truncate_respects_utf8_boundaries() {
        let s = "a\u{1F600}b"; // 'a' + 4-byte emoji + 'b'
        let out = truncate(s, 2);
        assert!(out.starts_with('a'), "out={out}");
        assert!(out.contains("truncated"), "out={out}");
        assert!(!out.contains('\u{1F600}'), "out={out}");
        assert_eq!(truncate(s, s.len()), s);
    }

    #[test]
    fn result_compile_error_emits_null_run() {
        let json = result(
            "test",
            CompileReport {
                ok: false,
                diagnostics: "1:1: error: boom",
                ast_preview: "",
                elapsed_ms: 3,
            },
            None,
        );
        assert_eq!(
            json,
            r#"{"ok":false,"backend":"test","compile":{"ok":false,"stderr":"1:1: error: boom","astPreview":"","elapsedMs":3},"run":null}"#
        );
    }

    #[test]
    fn result_run_success_is_overall_ok() {
        let json = result(
            "test",
            CompileReport {
                ok: true,
                diagnostics: "",
                ast_preview: "{}",
                elapsed_ms: 1,
            },
            Some(RunReport {
                ok: true,
                stdout: "hi\n",
                stderr: "",
                elapsed_ms: 2,
            }),
        );
        assert_eq!(
            json,
            r#"{"ok":true,"backend":"test","compile":{"ok":true,"stderr":"","astPreview":"{}","elapsedMs":1},"run":{"ok":true,"stdout":"hi\n","stderr":"","elapsedMs":2}}"#
        );
    }

    #[test]
    fn result_run_failure_is_not_overall_ok() {
        let json = result(
            "test",
            CompileReport {
                ok: true,
                diagnostics: "",
                ast_preview: "{}",
                elapsed_ms: 1,
            },
            Some(RunReport {
                ok: false,
                stdout: "",
                stderr: "error: trap",
                elapsed_ms: 2,
            }),
        );
        assert_eq!(
            json,
            r#"{"ok":false,"backend":"test","compile":{"ok":true,"stderr":"","astPreview":"{}","elapsedMs":1},"run":{"ok":false,"stdout":"","stderr":"error: trap","elapsedMs":2}}"#
        );
    }
}
