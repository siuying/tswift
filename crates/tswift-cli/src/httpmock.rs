//! Loading a scripted HTTP mock transport from a JSON route file.
//!
//! `tswift run` installs a [`MockHttpTransport`] when `TSWIFT_HTTP_MOCK`
//! points at a route file, so golden fixtures can exercise `URLSession`
//! deterministically and offline. The file is a JSON array of routes:
//!
//! ```json
//! [
//!   {"method": "GET", "url": "https://api.example.com/items",
//!    "status": 200, "headers": {"Content-Type": "application/json"},
//!    "body": "[1, 2, 3]"},
//!   {"method": "GET", "url": "https://down.example.com/",
//!    "error": "timedOut"}
//! ]
//! ```
//!
//! `body` is UTF-8 text; `error` is a `URLError.Code` case name. Requests not
//! matching any route fail like an unknown host (see `MockHttpTransport`).
//!
//! A route with a `chunksBase64` array field becomes a chunked route that
//! drives delegate callbacks and progress in M3+:
//!
//! ```json
//! [
//!   {"method": "GET", "url": "https://stream.example.com/data",
//!    "status": 200, "headers": {"Content-Type": "application/octet-stream"},
//!    "chunksBase64": ["Y2h1bmsxAA==", "Y2h1bmsyAA=="]},
//!   {"method": "GET", "url": "https://flaky.example.com/",
//!    "status": 200, "chunksBase64": ["cGFydA=="],
//!    "failAfterChunks": "networkConnectionLost"}
//! ]
//! ```

use tswift_core::json::{self, Json};
use tswift_core::{HttpError, HttpResponse, MockChunkedRoute, MockHttpTransport, MockRoute};

/// Parse the route file at `path` into a transport.
pub fn load(path: &str) -> Result<MockHttpTransport, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read `{path}`: {e}"))?;
    parse_routes(&text).map_err(|e| format!("invalid mock routes in `{path}`: {e}"))
}

fn parse_routes(text: &str) -> Result<MockHttpTransport, String> {
    let Json::Array(entries) = json::parse(text)? else {
        return Err("expected a top-level JSON array of routes".into());
    };
    let mut routes = Vec::new();
    let mut chunked_routes = Vec::new();
    for entry in &entries {
        if entry.get("chunksBase64").is_some() {
            chunked_routes.push(parse_chunked_route(entry)?);
        } else {
            routes.push(parse_route(entry)?);
        }
    }
    Ok(MockHttpTransport::new(routes).with_chunked_routes(chunked_routes))
}

/// Parse a chunked route (has `chunksBase64` field).
fn parse_chunked_route(entry: &Json) -> Result<MockChunkedRoute, String> {
    let method = match entry.get("method") {
        Some(Json::Str(m)) => m.clone(),
        None => "GET".to_string(),
        _ => return Err("route `method` must be a string".into()),
    };
    let Some(Json::Str(url)) = entry.get("url") else {
        return Err("route needs a string `url`".into());
    };
    let status = match entry.get("status") {
        Some(Json::Int(s)) => *s,
        None => 200,
        _ => return Err("route `status` must be an integer".into()),
    };
    let mut headers = Vec::new();
    match entry.get("headers") {
        Some(Json::Object(fields)) => {
            for (k, v) in fields {
                let Json::Str(v) = v else {
                    return Err("route header values must be strings".into());
                };
                headers.push((k.clone(), v.clone()));
            }
        }
        None => {}
        Some(_) => return Err("route `headers` must be an object".into()),
    }
    let chunks = match entry.get("chunksBase64") {
        Some(Json::Array(items)) => {
            let mut out = Vec::new();
            for item in items {
                let Json::Str(b64) = item else {
                    return Err("chunksBase64 entries must be base64 strings".into());
                };
                let bytes = tswift_core::base64::decode(b64)
                    .ok_or_else(|| format!("chunksBase64 entry is not valid base64: {b64}"))?;
                out.push(bytes);
            }
            out
        }
        _ => return Err("chunksBase64 must be a JSON array of base64 strings".into()),
    };
    let fail_after_chunks = match entry.get("failAfterChunks") {
        Some(Json::Str(code)) => Some((code.clone(), "scripted mid-stream failure".to_string())),
        None => None,
        Some(_) => return Err("failAfterChunks must be a URLError.Code case string".into()),
    };
    Ok(MockChunkedRoute {
        method,
        url: url.clone(),
        status,
        headers,
        chunks,
        fail_after_chunks,
    })
}

fn parse_route(entry: &Json) -> Result<MockRoute, String> {
    let method = match entry.get("method") {
        Some(Json::Str(m)) => m.clone(),
        None => "GET".to_string(),
        _ => return Err("route `method` must be a string".into()),
    };
    let Some(Json::Str(url)) = entry.get("url") else {
        return Err("route needs a string `url`".into());
    };
    let outcome = match entry.get("error") {
        Some(Json::Str(code)) => Err(HttpError::failed(code.clone(), "scripted failure")),
        Some(_) => return Err("route `error` must be a URLError.Code case string".into()),
        None => {
            let status = match entry.get("status") {
                Some(Json::Int(s)) => *s,
                None => 200,
                _ => return Err("route `status` must be an integer".into()),
            };
            let mut headers = Vec::new();
            match entry.get("headers") {
                Some(Json::Object(fields)) => {
                    for (k, v) in fields {
                        let Json::Str(v) = v else {
                            return Err("route header values must be strings".into());
                        };
                        headers.push((k.clone(), v.clone()));
                    }
                }
                None => {}
                Some(_) => return Err("route `headers` must be an object".into()),
            }
            let body = match entry.get("body") {
                Some(Json::Str(b)) => b.clone().into_bytes(),
                None => Vec::new(),
                Some(_) => return Err("route `body` must be a string".into()),
            };
            Ok(HttpResponse {
                status,
                headers,
                body,
            })
        }
    };
    Ok(MockRoute {
        method,
        url: url.clone(),
        outcome,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tswift_core::{HttpRequest, HttpTransport};

    fn get(url: &str) -> HttpRequest {
        HttpRequest {
            url: url.into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: None,
            timeout_seconds: 60.0,
        }
    }

    #[test]
    fn parses_success_and_error_routes() {
        let mut transport = parse_routes(
            r#"[
                {"method": "GET", "url": "https://a.example/x",
                 "status": 201, "headers": {"Content-Type": "text/plain"}, "body": "ok"},
                {"url": "https://b.example/", "error": "timedOut"}
            ]"#,
        )
        .unwrap();
        let ok = transport.perform(&get("https://a.example/x")).unwrap();
        assert_eq!(ok.status, 201);
        assert_eq!(ok.body, b"ok");
        let err = transport.perform(&get("https://b.example/")).unwrap_err();
        assert!(matches!(err, HttpError::Failed { code, .. } if code == "timedOut"));
    }

    #[test]
    fn rejects_malformed_route_files() {
        assert!(parse_routes(r#"{"not": "an array"}"#).is_err());
        assert!(parse_routes(r#"[{"method": "GET"}]"#).is_err());
    }

    #[test]
    fn parses_chunked_route_delivers_multiple_chunks_then_done() {
        // "aGk=" = base64("hi"), "Ynll" = base64("bye")
        let mut transport = parse_routes(
            r#"[
                {"method": "GET", "url": "https://stream.example.com/data",
                 "status": 200, "headers": {"Content-Type": "text/plain"},
                 "chunksBase64": ["aGk=", "Ynll"]}
            ]"#,
        )
        .unwrap();
        use tswift_core::http::HttpEvent;
        let h = transport
            .start(&get("https://stream.example.com/data"))
            .unwrap();
        let mut events = Vec::new();
        loop {
            let e = transport.next_event(h);
            let terminal = e.is_terminal();
            events.push(e);
            if terminal {
                break;
            }
        }
        // Response + 2 chunks + Done
        assert_eq!(events.len(), 4);
        assert!(matches!(
            &events[0],
            HttpEvent::Response { status: 200, .. }
        ));
        assert_eq!(events[1], HttpEvent::Chunk(b"hi".to_vec()));
        assert_eq!(events[2], HttpEvent::Chunk(b"bye".to_vec()));
        assert_eq!(events[3], HttpEvent::Done);
    }

    #[test]
    fn parses_chunked_route_with_fail_after_chunks() {
        // "cGFydA==" = base64("part")
        let mut transport = parse_routes(
            r#"[
                {"url": "https://flaky.example.com/",
                 "status": 200, "chunksBase64": ["cGFydA=="],
                 "failAfterChunks": "networkConnectionLost"}
            ]"#,
        )
        .unwrap();
        use tswift_core::http::HttpEvent;
        let h = transport.start(&get("https://flaky.example.com/")).unwrap();
        let mut events = Vec::new();
        loop {
            let e = transport.next_event(h);
            let terminal = e.is_terminal();
            events.push(e);
            if terminal {
                break;
            }
        }
        // Response + chunk + Failed
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[0], HttpEvent::Response { .. }));
        assert_eq!(events[1], HttpEvent::Chunk(b"part".to_vec()));
        assert!(
            matches!(&events[2], HttpEvent::Failed { code, .. } if code == "networkConnectionLost")
        );
    }

    #[test]
    fn mixed_regular_and_chunked_routes_parse_correctly() {
        let mut transport = parse_routes(
            r#"[
                {"url": "https://a.example/", "status": 200, "body": "plain"},
                {"url": "https://b.example/", "status": 200, "chunksBase64": ["aGk="]}
            ]"#,
        )
        .unwrap();
        // Regular route served via perform
        let ok = transport.perform(&get("https://a.example/")).unwrap();
        assert_eq!(ok.body, b"plain");
        // Chunked route served via start/next_event
        use tswift_core::http::HttpEvent;
        let h = transport.start(&get("https://b.example/")).unwrap();
        let e0 = transport.next_event(h);
        assert!(matches!(e0, HttpEvent::Response { status: 200, .. }));
        let e1 = transport.next_event(h);
        assert_eq!(e1, HttpEvent::Chunk(b"hi".to_vec()));
        let e2 = transport.next_event(h);
        assert_eq!(e2, HttpEvent::Done);
    }
}
