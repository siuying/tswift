//! The real HTTPS transport behind `tswift run --allow-network`.
//!
//! One blocking `ureq` (rustls) call per [`HttpTransport::perform`] — a match
//! for the interpreter's synchronous transport seam (see `tswift_core::http`).
//! Non-2xx statuses are responses, not errors, mirroring `URLSession`; only
//! transport-level failures map onto `URLError.Code` case names.

use std::time::Duration;

use tswift_core::{HttpError, HttpRequest, HttpResponse, HttpTransport};

/// A real network transport; each request builds a per-timeout agent.
#[derive(Debug, Default)]
pub struct NetTransport;

impl HttpTransport for NetTransport {
    fn perform(&mut self, req: &HttpRequest) -> Result<HttpResponse, HttpError> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs_f64(
                req.timeout_seconds.clamp(0.001, 86_400.0),
            )))
            .http_status_as_error(false)
            .build()
            .into();

        let mut builder = ureq::http::Request::builder()
            .method(req.method.as_str())
            .uri(&req.url);
        for (name, value) in &req.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        let request = builder
            .body(req.body.clone().unwrap_or_default())
            .map_err(|e| HttpError::failed("badURL", e.to_string()))?;

        let mut response = agent.run(request).map_err(translate)?;
        let status = i64::from(response.status().as_u16());
        let headers = response
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_string(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .collect();
        let body = response
            .body_mut()
            .read_to_vec()
            .map_err(|e| HttpError::failed("cannotDecodeRawData", e.to_string()))?;
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

/// Map a `ureq` failure onto the closest `URLError.Code` case.
fn translate(e: ureq::Error) -> HttpError {
    let code = match &e {
        ureq::Error::Timeout(_) => "timedOut",
        ureq::Error::HostNotFound => "cannotFindHost",
        ureq::Error::ConnectionFailed => "cannotConnectToHost",
        ureq::Error::BadUri(_) => "badURL",
        ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => "httpTooManyRedirects",
        ureq::Error::Tls(_) | ureq::Error::TlsRequired => "secureConnectionFailed",
        ureq::Error::Protocol(_) => "cannotParseResponse",
        ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => "timedOut",
        ureq::Error::Io(_) => "networkConnectionLost",
        _ => "cannotParseResponse",
    };
    HttpError::failed(code, e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_maps_transport_failures_to_url_error_cases() {
        assert!(matches!(
            translate(ureq::Error::HostNotFound),
            HttpError::Failed { code, .. } if code == "cannotFindHost"
        ));
        assert!(matches!(
            translate(ureq::Error::ConnectionFailed),
            HttpError::Failed { code, .. } if code == "cannotConnectToHost"
        ));
        assert!(matches!(
            translate(ureq::Error::TooManyRedirects),
            HttpError::Failed { code, .. } if code == "httpTooManyRedirects"
        ));
    }

    #[test]
    fn perform_refuses_a_connection_on_an_unroutable_port() {
        // Port 1 on loopback is essentially never listening; the transport
        // must fail with a URLError-shaped code, not panic or hang.
        let mut t = NetTransport;
        let err = t
            .perform(&HttpRequest {
                url: "http://127.0.0.1:1/".into(),
                method: "GET".into(),
                headers: Vec::new(),
                body: None,
                timeout_seconds: 2.0,
            })
            .unwrap_err();
        assert!(matches!(err, HttpError::Failed { .. }), "got {err:?}");
    }
}
