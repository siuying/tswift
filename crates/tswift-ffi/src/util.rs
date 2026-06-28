//! Serialization helpers shared by the `extern "C"` entry points.
//!
//! Ported from `tswift-wasm` so the native boundary produces byte-compatible
//! JSON. We avoid `serde_json` (offline constraint; see `tswift-core::json`).

/// JSON-escape a string body (no surrounding quotes).
pub(crate) fn escape_json(value: &str) -> String {
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

/// Truncate a value to `max` bytes on a UTF-8 boundary, appending a marker.
pub(crate) fn truncate(value: &str, max: usize) -> String {
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

/// Milliseconds since the Unix epoch (native clock).
pub(crate) fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

/// Elapsed whole milliseconds since `started`.
pub(crate) fn elapsed_ms(started: f64) -> u64 {
    (now_ms() - started).max(0.0).round() as u64
}
