//! Timing helpers for the `extern "C"` entry points.
//!
//! The pure JSON shaping (escape/truncate/envelope) lives in
//! `tswift-core::result_json`; only the native clock stays here because the wasm
//! host reads time from a different source.

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
