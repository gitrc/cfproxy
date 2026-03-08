use chrono::Utc;
use serde_json::{json, Value};

use crate::event::HttpRequest;

/// Export requests to HAR 1.2 JSON format.
pub fn to_har(requests: &[&HttpRequest], creator_version: &str) -> Value {
    json!({
        "log": {
            "version": "1.2",
            "creator": {
                "name": "cfproxy",
                "version": creator_version,
            },
            "entries": requests.iter().map(|req| entry(req)).collect::<Vec<_>>(),
        }
    })
}

fn entry(req: &HttpRequest) -> Value {
    json!({
        "startedDateTime": Utc::now().to_rfc3339(),
        "time": req.duration.as_millis() as f64,
        "request": {
            "method": req.method,
            "url": req.path,
            "httpVersion": "HTTP/1.1",
            "headers": headers_to_har(&req.request_headers),
            "queryString": [],
            "bodySize": req.request_size,
            "postData": req.request_body.as_ref().map(|b| json!({
                "mimeType": find_content_type(&req.request_headers)
                    .unwrap_or_else(|| "application/octet-stream".to_string()),
                "text": b,
            })),
        },
        "response": {
            "status": req.status,
            "statusText": status_text(req.status),
            "httpVersion": "HTTP/1.1",
            "headers": headers_to_har(&req.response_headers),
            "content": {
                "size": req.response_size,
                "mimeType": find_content_type(&req.response_headers)
                    .unwrap_or_else(|| "application/octet-stream".to_string()),
                "text": req.response_body,
            },
            "bodySize": req.response_size,
        },
        "timings": {
            "send": 0,
            "wait": req.duration.as_millis() as f64,
            "receive": 0,
        },
    })
}

fn headers_to_har(headers: &[crate::event::HeaderPair]) -> Vec<Value> {
    headers
        .iter()
        .map(|h| {
            json!({
                "name": h.name,
                "value": h.value,
            })
        })
        .collect()
}

fn find_content_type(headers: &[crate::event::HeaderPair]) -> Option<String> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.clone())
}

fn status_text(code: u16) -> &'static str {
    match code {
        100 => "Continue",
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{HeaderPair, HttpRequest};
    use std::time::Duration;

    fn sample_request() -> HttpRequest {
        HttpRequest {
            method: "GET".into(),
            path: "/api/test".into(),
            status: 200,
            duration: Duration::from_millis(42),
            remote_ip: Some("1.2.3.4".into()),
            country: Some("US".into()),
            user_agent: Some("curl/8.1".into()),
            request_headers: vec![HeaderPair {
                name: "content-type".into(),
                value: "application/json".into(),
            }],
            response_headers: vec![HeaderPair {
                name: "content-type".into(),
                value: "application/json".into(),
            }],
            request_size: 0,
            response_size: 13,
            request_body: None,
            response_body: Some("{\"ok\":true}".into()),
            is_websocket: false,
            ws_frames: Vec::new(),
            is_mock: false,
            timestamp: chrono::Local::now(),
        }
    }

    #[test]
    fn har_has_correct_structure() {
        let req = sample_request();
        let har = to_har(&[&req], "0.1.0");

        assert_eq!(har["log"]["version"], "1.2");
        assert_eq!(har["log"]["creator"]["name"], "cfproxy");
        assert_eq!(har["log"]["creator"]["version"], "0.1.0");
        assert!(har["log"]["entries"].is_array());
        assert_eq!(har["log"]["entries"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn har_entry_contains_request_data() {
        let req = sample_request();
        let har = to_har(&[&req], "0.1.0");
        let entry = &har["log"]["entries"][0];

        assert_eq!(entry["request"]["method"], "GET");
        assert_eq!(entry["request"]["url"], "/api/test");
        assert_eq!(entry["response"]["status"], 200);
        assert_eq!(entry["response"]["statusText"], "OK");
        assert_eq!(entry["time"], 42.0);
        assert_eq!(entry["timings"]["wait"], 42.0);
    }

    #[test]
    fn har_with_empty_requests() {
        let har = to_har(&[], "0.1.0");

        assert_eq!(har["log"]["entries"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn har_with_body() {
        let mut req = sample_request();
        req.method = "POST".into();
        req.request_body = Some("{\"data\":1}".into());
        req.request_size = 10;

        let har = to_har(&[&req], "0.1.0");
        let entry = &har["log"]["entries"][0];

        let post_data = &entry["request"]["postData"];
        assert_eq!(post_data["mimeType"], "application/json");
        assert_eq!(post_data["text"], "{\"data\":1}");

        let content = &entry["response"]["content"];
        assert_eq!(content["text"], "{\"ok\":true}");
        assert_eq!(content["mimeType"], "application/json");
    }

    #[test]
    fn status_text_known_codes() {
        assert_eq!(status_text(200), "OK");
        assert_eq!(status_text(404), "Not Found");
        assert_eq!(status_text(500), "Internal Server Error");
        assert_eq!(status_text(301), "Moved Permanently");
        assert_eq!(status_text(204), "No Content");
        assert_eq!(status_text(999), "");
    }
}
