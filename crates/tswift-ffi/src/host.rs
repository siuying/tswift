//! Host-native function registration for the native embedding (Epic #246).
//!
//! A *host function* is a native (Swift/C) function that interpreted Swift can
//! call by name. The host registers one with [`tswift_register_host_fn`],
//! supplying a compact JSON signature (see [`tswift_core::host_bridge`]), a C
//! callback, and an opaque `userdata` — exactly the retain/release/userdata
//! contract used by the one-shot HTTP handler ([`crate::http::HostHttpHandler`]).
//!
//! The boundary is *synchronous* (the interpreter is a cooperative
//! single-threaded executor, ADR-0005): when interpreted code calls the
//! function, Rust validates and encodes the arguments to JSON, invokes the C
//! callback, and the callback **must** call [`tswift_host_respond`] exactly once
//! with the result JSON before returning. Rust then decodes and validates the
//! result against the declared return type.
//!
//! Registrations live on the [`Context`][crate::Context] and are installed into
//! the interpreter in both execution paths (one-shot run and SwiftUI compile),
//! the same place the HTTP transport is wired.

use std::ffi::{c_char, c_void, CString};
use std::sync::Arc;

use tswift_core::host_bridge::HostCallHandler;
use tswift_core::{HostSignature, Interpreter};

/// The C callback signature: `(userdata, name, args_json, call)`. `name` is the
/// called function's name; `args_json` is a JSON array of already-validated
/// arguments in declared order. The callback MUST invoke
/// [`tswift_host_respond`]`(call, result_json)` exactly once before returning.
pub type TswiftHostFn = unsafe extern "C" fn(
    userdata: *mut c_void,
    name: *const c_char,
    args_json: *const c_char,
    call: *mut c_void,
);

/// A registered host function: the C callback plus its opaque userdata.
#[derive(Clone, Copy)]
pub(crate) struct HostFnHandler {
    pub(crate) callback: TswiftHostFn,
    pub(crate) userdata: *mut c_void,
}

// SAFETY: the callback fn pointer + userdata lifetime contract is enforced by
// the C caller (must outlive the context). The interpreter is single-threaded
// (ADR-0005), so concurrent access to `userdata` via this type never occurs.
unsafe impl Send for HostFnHandler {}
unsafe impl Sync for HostFnHandler {}

/// The per-call result slot [`tswift_host_respond`] writes into. The `call`
/// token handed to the host is a pointer to this.
struct ResultSlot {
    result_json: Option<String>,
}

/// Copy `result_json` into the in-flight call `call`. Calling it outside the
/// callback, or twice, is undefined.
///
/// # Safety
/// `call` must be the token passed to the currently-executing host-function
/// callback and `result_json` a valid NUL-terminated C string (or null, which
/// is ignored). The token is only valid for the duration of the callback.
#[no_mangle]
pub unsafe extern "C" fn tswift_host_respond(call: *mut c_void, result_json: *const c_char) {
    if call.is_null() {
        return;
    }
    let Some(text) = crate::borrow_str(result_json) else {
        return;
    };
    let slot = &mut *call.cast::<ResultSlot>();
    slot.result_json = Some(text.to_string());
}

impl HostCallHandler for HostFnHandler {
    fn call(&self, name: &str, args_json: &str) -> Result<String, String> {
        let c_name =
            CString::new(name).map_err(|_| "host fn name contains a NUL byte".to_string())?;
        let c_args = CString::new(args_json)
            .map_err(|_| "host fn args JSON contains a NUL byte".to_string())?;
        let mut slot = ResultSlot { result_json: None };
        // SAFETY: `callback`/`userdata` were registered together by the host
        // via `tswift_register_host_fn`; the slot pointer outlives the call.
        unsafe {
            (self.callback)(
                self.userdata,
                c_name.as_ptr(),
                c_args.as_ptr(),
                (&mut slot as *mut ResultSlot).cast(),
            );
        }
        slot.result_json
            .ok_or_else(|| "host function callback returned without responding".to_string())
    }
}

/// One host-function registration stored on the [`Context`][crate::Context]:
/// its name, signature JSON, and the retained handler.
pub(crate) struct HostFnRegistration {
    pub(crate) name: String,
    pub(crate) signature_json: String,
    pub(crate) handler: Arc<HostFnHandler>,
}

/// Validate `signature_json`, then register (or replace, by name) a host
/// function backed by `callback`/`userdata` in `regs`. Returns the registered
/// function name, or an error string if the signature is malformed.
pub(crate) fn register(
    regs: &mut Vec<HostFnRegistration>,
    signature_json: &str,
    callback: TswiftHostFn,
    userdata: *mut c_void,
) -> Result<String, String> {
    let signature = HostSignature::from_json(signature_json)?;
    let name = signature.name.clone();
    let handler = Arc::new(HostFnHandler { callback, userdata });
    // Replace any prior registration with the same name.
    regs.retain(|r| r.name != name);
    regs.push(HostFnRegistration {
        name: name.clone(),
        signature_json: signature_json.to_string(),
        handler,
    });
    Ok(name)
}

/// Remove the host function named `name` from `regs` (a no-op if absent).
pub(crate) fn remove(regs: &mut Vec<HostFnRegistration>, name: &str) {
    regs.retain(|r| r.name != name);
}

/// Install every registered host function into `interp` before it runs user
/// source. Called from both the one-shot run and the SwiftUI compile paths.
pub(crate) fn install(interp: &mut Interpreter, regs: &[HostFnRegistration]) {
    for reg in regs {
        // The signature was validated at registration time; a late failure here
        // would be an internal invariant break, so ignore the (unreachable) Err.
        let _ = interp.register_host_fn(&reg.signature_json, Some(reg.handler.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    /// A callback that echoes the sum of two ints: sum(_ a: Int, _ b: Int) -> Int.
    unsafe extern "C" fn sum_callback(
        _userdata: *mut c_void,
        _name: *const c_char,
        args_json: *const c_char,
        call: *mut c_void,
    ) {
        let args = CStr::from_ptr(args_json).to_str().unwrap();
        let tswift_core::json::Json::Array(items) = tswift_core::json::parse(args).unwrap() else {
            panic!("expected array");
        };
        let (tswift_core::json::Json::Int(a), tswift_core::json::Json::Int(b)) =
            (&items[0], &items[1])
        else {
            panic!("expected two ints");
        };
        let reply = CString::new(format!("{}", a + b)).unwrap();
        tswift_host_respond(call, reply.as_ptr());
    }

    const SUM_SIG: &str =
        r#"{"name":"sum","params":[{"type":"Int"},{"type":"Int"}],"returns":"Int"}"#;

    #[test]
    fn register_and_install_into_interpreter() {
        let mut regs = Vec::new();
        let name = register(&mut regs, SUM_SIG, sum_callback, std::ptr::null_mut()).unwrap();
        assert_eq!(name, "sum");

        // `install` re-registers each entry on a fresh interpreter without
        // panicking; the end-to-end call is exercised through `tswift_run` in
        // the lib-level FFI tests. Here we just verify the wiring path is sound.
        let mut out = Vec::new();
        let mut interp = Interpreter::new(&mut out);
        install(&mut interp, &regs);
    }

    #[test]
    fn register_replaces_by_name() {
        let mut regs = Vec::new();
        register(&mut regs, SUM_SIG, sum_callback, std::ptr::null_mut()).unwrap();
        register(&mut regs, SUM_SIG, sum_callback, std::ptr::null_mut()).unwrap();
        assert_eq!(regs.len(), 1, "same-name registration should replace");
    }

    #[test]
    fn remove_drops_registration() {
        let mut regs = Vec::new();
        register(&mut regs, SUM_SIG, sum_callback, std::ptr::null_mut()).unwrap();
        remove(&mut regs, "sum");
        assert!(regs.is_empty());
    }

    #[test]
    fn register_rejects_malformed_signature() {
        let mut regs = Vec::new();
        let err = register(
            &mut regs,
            r#"{"params":[]}"#,
            sum_callback,
            std::ptr::null_mut(),
        )
        .unwrap_err();
        assert!(err.contains("name"), "{err}");
    }

    #[test]
    fn respond_null_call_is_noop() {
        unsafe { tswift_host_respond(std::ptr::null_mut(), std::ptr::null()) };
    }
}
