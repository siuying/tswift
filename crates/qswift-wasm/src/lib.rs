#![forbid(unsafe_code)]

use qswift_core::Interpreter;
use qswift_frontend::Analysis;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = runSwift)]
pub fn run_swift(source: &str) -> String {
    install_panic_hook();
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
    qswift_std::install(&mut interp);
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

    format!(
        "{}\n\n[prototype truncated {} bytes]",
        &value[..max],
        value.len() - max
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

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = performance, js_name = now)]
    fn performance_now() -> f64;
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn console_error(msg: &str);
}

fn now_ms() -> f64 {
    performance_now()
}

/// Forward Rust panics to `console.error` so the browser shows a real message
/// instead of an opaque `RuntimeError: unreachable`.
fn install_panic_hook() {
    use std::sync::Once;
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            console_error(&format!("qswift-wasm panic: {info}"));
        }));
    });
}

fn elapsed_ms(started: f64) -> u64 {
    (now_ms() - started).max(0.0).round() as u64
}
