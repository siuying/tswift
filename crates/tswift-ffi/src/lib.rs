//! C-ABI native embedding host for tswift.
//!
//! Exposes a small `extern "C"` surface fronted by two Swift façades
//! (`TSwiftCore`, `TSwiftUI`). The boundary is *serialized*: every value that
//! crosses the ABI is a JSON string the caller must release with
//! [`tswift_string_free`]. See `docs/plan/native-host.md` and CONTEXT.md.
//!
//! All `unsafe` for the FFI lives in this crate, preserving ADR-0001's
//! FFI-only-unsafe rule.

use std::ffi::{c_char, CString};

/// The lifespan-owning VM handle handed to C as an opaque pointer — the native
/// analogue of QuickJS's `JSContext`. Owns the reclaimable interpreter bundle
/// (grown in later tasks: one-shot run state, then the SwiftUI render session).
/// Created with [`tswift_context_new`] and freed with [`tswift_context_free`].
pub struct Context {
    // Persistent state is added by T2 (`tswift_run`) and T3 (the SwiftUI render
    // session). The handle's T1 job is lifespan ownership and the `unsafe` seam.
    _private: (),
}

impl Context {
    fn new() -> Self {
        Context { _private: () }
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
// Exercised by the T1 round-trip test; the returning entry points that consume
// it land in T2 (`tswift_run`) and T3 (the SwiftUI session).
#[cfg_attr(not(test), allow(dead_code))]
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

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
}
