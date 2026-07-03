//! Host-injected HTTP transport for the native embedding (the `URLSession`
//! seam across the C ABI).
//!
//! The interpreter's transport seam is synchronous (see `tswift_core::http`),
//! so the C contract is too: the host registers one handler with
//! [`tswift_set_http_handler`]; each script request invokes it with a
//! request-JSON string and an opaque `call` token, and the handler must call
//! [`tswift_http_respond`] with a response-JSON string **before returning**
//! (it may block internally — e.g. a semaphore around a real `URLSession`
//! task). No pointer ownership crosses the boundary: Rust copies the response
//! during the `respond` call.
//!
//! Request JSON:  `{"url", "method", "headers": [[k, v]...],
//!                  "timeoutSeconds", "bodyBase64"?}`
//! Response JSON: `{"status", "headers": [[k, v]...], "bodyBase64"?}`
//!            or  `{"error": "<URLError.Code case>", "message"?}`

use std::ffi::{c_char, c_void, CString};

use tswift_core::http::{decode_response_json, encode_request_json};
use tswift_core::{HttpError, HttpRequest, HttpResponse, HttpTransport};

/// The C handler signature: `(userdata, request_json, call)`. Must invoke
/// `tswift_http_respond(call, response_json)` exactly once before returning.
pub type TswiftHttpHandler =
    unsafe extern "C" fn(userdata: *mut c_void, request_json: *const c_char, call: *mut c_void);

/// A registered host handler: the function pointer plus its opaque userdata.
#[derive(Clone, Copy)]
pub(crate) struct HostHttpHandler {
    pub(crate) handler: TswiftHttpHandler,
    pub(crate) userdata: *mut c_void,
}

/// The per-call response slot `tswift_http_respond` writes into. The `call`
/// token handed to the host is a pointer to this.
struct ResponseSlot {
    response_json: Option<String>,
}

/// Copy `response_json` into the in-flight call `call`. See the C header for
/// the full contract; calling it outside the handler, or twice, is undefined.
///
/// # Safety
/// `call` must be the token passed to the currently-executing handler and
/// `response_json` a valid NUL-terminated C string (or null, which is
/// ignored). The token is only valid for the duration of the handler call.
#[no_mangle]
pub unsafe extern "C" fn tswift_http_respond(call: *mut c_void, response_json: *const c_char) {
    if call.is_null() {
        return;
    }
    let Some(text) = crate::borrow_str(response_json) else {
        return;
    };
    let slot = &mut *call.cast::<ResponseSlot>();
    slot.response_json = Some(text.to_string());
}

impl HttpTransport for HostHttpHandler {
    fn perform(&mut self, req: &HttpRequest) -> Result<HttpResponse, HttpError> {
        let request_json = encode_request_json(req);
        let c_request = CString::new(request_json)
            .map_err(|_| HttpError::failed("badURL", "request contains a NUL byte"))?;
        let mut slot = ResponseSlot {
            response_json: None,
        };
        // SAFETY: `handler`/`userdata` were registered together by the host
        // via `tswift_set_http_handler`; the slot pointer outlives the call.
        unsafe {
            (self.handler)(
                self.userdata,
                c_request.as_ptr(),
                (&mut slot as *mut ResponseSlot).cast(),
            );
        }
        let Some(response_json) = slot.response_json else {
            return Err(HttpError::failed(
                "badServerResponse",
                "host HTTP handler returned without responding",
            ));
        };
        decode_response_json(&response_json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> HttpRequest {
        HttpRequest {
            url: "https://example.com/a".into(),
            method: "POST".into(),
            headers: vec![("Content-Type".into(), "text/plain".into())],
            body: Some(b"hi".to_vec()),
            timeout_seconds: 30.0,
        }
    }

    #[test]
    fn handler_round_trips_through_the_c_surface() {
        unsafe extern "C" fn echo_handler(
            _userdata: *mut c_void,
            request_json: *const c_char,
            call: *mut c_void,
        ) {
            // Assert the request arrived, then respond in-line like a host.
            let req = std::ffi::CStr::from_ptr(request_json).to_str().unwrap();
            assert!(req.contains("https://example.com/a"));
            let response =
                CString::new(r#"{"status": 201, "headers": [], "bodyBase64": "b2s="}"#).unwrap();
            tswift_http_respond(call, response.as_ptr());
        }
        let mut transport = HostHttpHandler {
            handler: echo_handler,
            userdata: std::ptr::null_mut(),
        };
        let resp = transport.perform(&request()).unwrap();
        assert_eq!(resp.status, 201);
        assert_eq!(resp.body, b"ok");
    }

    #[test]
    fn silent_handler_maps_to_bad_server_response() {
        unsafe extern "C" fn silent_handler(
            _userdata: *mut c_void,
            _request_json: *const c_char,
            _call: *mut c_void,
        ) {
        }
        let mut transport = HostHttpHandler {
            handler: silent_handler,
            userdata: std::ptr::null_mut(),
        };
        let err = transport.perform(&request()).unwrap_err();
        assert!(matches!(err, HttpError::Failed { code, .. } if code == "badServerResponse"));
    }
}
