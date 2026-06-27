#![forbid(unsafe_code)]

use tswift_core::Interpreter;
use tswift_frontend::Analysis;
use wasm_bindgen::prelude::*;

/// Compile and run a single Swift source string, returning a JSON result.
///
/// This is the wasm entry point. The heavy lifting lives in [`run_swift_impl`],
/// which is platform-independent and exercised by the native unit tests.
#[wasm_bindgen(js_name = runSwift)]
pub fn run_swift(source: &str) -> String {
    install_panic_hook();
    run_swift_impl(source)
}

fn run_swift_impl(source: &str) -> String {
    let started = now_ms();

    let analysis = match Analysis::analyze(source, "main.swift") {
        Ok(analysis) => analysis,
        Err(error) => {
            return format!(
                "{{\"ok\":false,\"backend\":\"wasm\",\"compile\":{{\"ok\":false,\"stderr\":\"{}\",\"astPreview\":\"\",\"elapsedMs\":{}}},\"run\":null}}",
                escape_json(&error.to_string()),
                elapsed_ms(started)
            );
        }
    };

    let mut diagnostics = String::new();
    for diagnostic in analysis.diagnostics() {
        diagnostics.push_str(&format!(
            "{}:{}: {}\n",
            diagnostic.line, diagnostic.col, diagnostic.message
        ));
    }

    let ast_preview = analysis.root().dump_json();
    let compile_elapsed = elapsed_ms(started);

    let run_started = now_ms();
    let analysis: &'static Analysis = Box::leak(Box::new(analysis));
    let mut stdout = Vec::new();
    let mut interp = Interpreter::new(&mut stdout);
    tswift_std::install(&mut interp);
    tswift_foundation::install(&mut interp);
    interp.set_filename("main.swift");

    let run_result = interp.run(analysis);
    let run_elapsed = elapsed_ms(run_started);
    let stdout = String::from_utf8_lossy(&stdout);

    match run_result {
        Ok(()) => format!(
            "{{\"ok\":true,\"backend\":\"wasm\",\"compile\":{{\"ok\":true,\"stderr\":\"{}\",\"astPreview\":\"{}\",\"elapsedMs\":{}}},\"run\":{{\"ok\":true,\"stdout\":\"{}\",\"stderr\":\"\",\"elapsedMs\":{}}}}}",
            escape_json(&diagnostics),
            escape_json(&truncate(&ast_preview, 6_000)),
            compile_elapsed,
            escape_json(&truncate(&stdout, 24_000)),
            run_elapsed
        ),
        Err(error) => format!(
            "{{\"ok\":false,\"backend\":\"wasm\",\"compile\":{{\"ok\":true,\"stderr\":\"{}\",\"astPreview\":\"{}\",\"elapsedMs\":{}}},\"run\":{{\"ok\":false,\"stdout\":\"{}\",\"stderr\":\"error: {}\",\"elapsedMs\":{}}}}}",
            escape_json(&diagnostics),
            escape_json(&truncate(&ast_preview, 6_000)),
            compile_elapsed,
            escape_json(&truncate(&stdout, 24_000)),
            escape_json(&error.to_string()),
            run_elapsed
        ),
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }

    // Slice on a UTF-8 char boundary at or before `max`; a raw byte slice would
    // panic when the limit falls inside a multibyte character.
    let mut end = max;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }

    format!(
        "{}\n\n[prototype truncated {} bytes]",
        &value[..end],
        value.len() - end
    )
}

fn escape_json(value: &str) -> String {
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

// ── Platform shims ──────────────────────────────────────────────────────────
// On wasm we call into JS (`performance.now`, `console.error`); on native
// (used by the unit tests below) we fall back to std equivalents so the crate
// compiles and runs off-target.

#[cfg(target_arch = "wasm32")]
mod platform {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = performance, js_name = now)]
        fn performance_now() -> f64;
        #[wasm_bindgen(js_namespace = console, js_name = error)]
        fn console_error(msg: &str);
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
fn install_panic_hook() {
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
        // Parse/semantic errors are recorded as diagnostics: analysis succeeds
        // structurally but the interpreter refuses to run.
        let json = run_swift_impl("let = = =");
        assert_eq!(bool_field(&json, "ok"), Some(false), "json={json}");
        assert!(json.contains("\"run\":{\"ok\":false"), "json={json}");
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
    fn truncate_respects_utf8_boundaries() {
        // A multibyte character straddling the limit must not panic; truncation
        // falls back to the previous char boundary.
        let s = "a\u{1F600}b"; // 'a' + 4-byte emoji + 'b'
                               // max=2 lands inside the emoji (bytes 1..5); expect only "a" kept.
        let out = truncate(s, 2);
        assert!(out.starts_with('a'), "out={out}");
        assert!(out.contains("truncated"), "out={out}");
        assert!(!out.contains('\u{1F600}'), "out={out}");
        // A limit on a boundary keeps the whole prefix.
        assert_eq!(truncate(s, s.len()), s);
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
}
