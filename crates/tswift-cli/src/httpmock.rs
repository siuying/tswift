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

use tswift_core::json::{self, Json};
use tswift_core::{HttpError, HttpResponse, MockHttpTransport, MockRoute};

/// Parse the route file at `path` into a transport.
pub fn load(path: &str) -> Result<MockHttpTransport, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read `{path}`: {e}"))?;
    parse_routes(&text).map_err(|e| format!("invalid mock routes in `{path}`: {e}"))
}

fn parse_routes(text: &str) -> Result<MockHttpTransport, String> {
    let Json::Array(entries) = json::parse(text)? else {
        return Err("expected a top-level JSON array of routes".into());
    };
    let routes = entries
        .iter()
        .map(parse_route)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(MockHttpTransport::new(routes))
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
}
