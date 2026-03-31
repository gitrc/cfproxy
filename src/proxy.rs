use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite;

use crate::error::Result;
use crate::event::{HeaderPair, HttpRequest, TunnelEvent, WebSocketFrame, WsDirection, WsOpcode};
use crate::mock::MockRules;

/// IP allowlist: when non-empty, only listed IPs may access the proxy.
pub type AllowIps = Arc<HashSet<String>>;

/// Start a local reverse proxy that forwards to `target_port` and logs requests.
/// Returns the port the proxy is listening on.
pub async fn start(
    target_port: u16,
    tx: mpsc::Sender<TunnelEvent>,
    auth: Option<(String, String)>,
    mock_rules: MockRules,
    allow_ips: Vec<String>,
) -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let proxy_port = listener.local_addr()?.port();
    let auth = auth.map(Arc::new);
    let allow_ips: AllowIps = Arc::new(allow_ips.into_iter().collect());

    tokio::spawn(async move {
        loop {
            let (stream, _addr) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => continue,
            };
            let tx = tx.clone();
            let auth = auth.clone();
            let mock_rules = mock_rules.clone();
            let allow_ips = allow_ips.clone();
            let io = TokioIo::new(stream);

            tokio::spawn(async move {
                let svc = service_fn(move |req| {
                    let tx = tx.clone();
                    let auth = auth.clone();
                    let mock_rules = mock_rules.clone();
                    let allow_ips = allow_ips.clone();
                    handle(req, target_port, tx, auth, mock_rules, allow_ips)
                });
                let conn = http1::Builder::new()
                    .serve_connection(io, svc)
                    .with_upgrades();
                if let Err(e) = conn.await {
                    tracing::debug!("proxy connection error: {}", e);
                }
            });
        }
    });

    Ok(proxy_port)
}

async fn handle(
    req: Request<Incoming>,
    target_port: u16,
    tx: mpsc::Sender<TunnelEvent>,
    auth: Option<Arc<(String, String)>>,
    mock_rules: MockRules,
    allow_ips: AllowIps,
) -> std::result::Result<Response<Full<Bytes>>, hyper::Error> {
    let start = Instant::now();
    let method = req.method().to_string();
    let path = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());

    let remote_ip = header_val(req.headers(), "cf-connecting-ip")
        .or_else(|| header_val(req.headers(), "x-forwarded-for"));
    let country = header_val(req.headers(), "cf-ipcountry");
    let user_agent = header_val(req.headers(), "user-agent");
    let request_headers = collect_headers(req.headers());

    // Check IP allowlist before anything else
    if !allow_ips.is_empty() {
        let allowed = remote_ip
            .as_ref()
            .map(|ip| allow_ips.contains(ip))
            .unwrap_or(false);
        if !allowed {
            let duration = start.elapsed();
            let _ = tx
                .send(TunnelEvent::HttpRequest(HttpRequest {
                    method,
                    path,
                    status: 403,
                    duration,
                    remote_ip,
                    country,
                    user_agent,
                    request_headers,
                    response_headers: Vec::new(),
                    request_size: 0,
                    response_size: 0,
                    request_body: None,
                    response_body: None,
                    is_websocket: false,
                    ws_frames: Vec::new(),
                    is_mock: false,
                    timestamp: chrono::Local::now(),
                }))
                .await;
            return Ok(Response::builder()
                .status(hyper::StatusCode::FORBIDDEN)
                .body(Full::new(Bytes::from("Forbidden")))
                .unwrap());
        }
    }

    // Check Basic Auth before forwarding
    if let Some(ref expected) = auth {
        let authorized = header_val(req.headers(), "authorization")
            .map(|val| {
                let expected_encoded = base64::engine::general_purpose::STANDARD
                    .encode(format!("{}:{}", expected.0, expected.1));
                let expected_header = format!("Basic {}", expected_encoded);
                val == expected_header
            })
            .unwrap_or(false);

        if !authorized {
            let duration = start.elapsed();
            let _ = tx
                .send(TunnelEvent::HttpRequest(HttpRequest {
                    method,
                    path,
                    status: 401,
                    duration,
                    remote_ip,
                    country,
                    user_agent,
                    request_headers,
                    response_headers: Vec::new(),
                    request_size: 0,
                    response_size: 0,
                    request_body: None,
                    response_body: None,
                    is_websocket: false,
                    ws_frames: Vec::new(),
                    is_mock: false,
                    timestamp: chrono::Local::now(),
                }))
                .await;
            return Ok(Response::builder()
                .status(hyper::StatusCode::UNAUTHORIZED)
                .header("WWW-Authenticate", "Basic realm=\"cfproxy\"")
                .body(Full::new(Bytes::from("Unauthorized")))
                .unwrap());
        }
    }

    // Check mock rules before forwarding
    {
        let mut rules = mock_rules.write().await;
        if let Some(rule) = rules.iter_mut().find(|r| r.matches(&method, &path)) {
            rule.hit_count += 1;
            let duration = start.elapsed();
            let body = rule.body.clone();
            let status = rule.status;
            let content_type = rule.content_type.clone();
            let response_body_preview = if body.is_empty() { None } else { Some(body.clone()) };
            let response_size = body.len() as u64;
            let _ = tx
                .send(TunnelEvent::HttpRequest(HttpRequest {
                    method,
                    path,
                    status,
                    duration,
                    remote_ip,
                    country,
                    user_agent,
                    request_headers,
                    response_headers: vec![crate::event::HeaderPair {
                        name: "content-type".to_string(),
                        value: content_type.clone(),
                    }],
                    request_size: 0,
                    response_size,
                    request_body: None,
                    response_body: response_body_preview,
                    is_websocket: false,
                    ws_frames: Vec::new(),
                    is_mock: true,
                    timestamp: chrono::Local::now(),
                }))
                .await;
            return Ok(Response::builder()
                .status(status)
                .header("content-type", content_type)
                .body(Full::new(Bytes::from(body)))
                .unwrap());
        }
    }

    // Detect WebSocket upgrade
    if is_websocket_upgrade(req.headers()) {
        return handle_websocket(req, target_port, &path, &method, start, remote_ip, country, user_agent, request_headers, tx).await;
    }

    let target_uri = format!("http://127.0.0.1:{}{}", target_port, path);

    match forward(req, &target_uri).await {
        Ok(fwd) => {
            let duration = start.elapsed();
            let mut resp = Response::builder().status(fwd.status);
            for (key, val) in fwd.headers.iter() {
                resp = resp.header(key, val);
            }
            let response_body_preview = body_preview(&fwd.body);
            let _ = tx
                .send(TunnelEvent::HttpRequest(HttpRequest {
                    method,
                    path,
                    status: fwd.status.as_u16(),
                    duration,
                    remote_ip,
                    country,
                    user_agent,
                    request_headers,
                    response_headers: collect_headers(&fwd.headers),
                    request_size: fwd.request_size,
                    response_size: fwd.body.len() as u64,
                    request_body: fwd.request_body_preview,
                    response_body: response_body_preview,
                    is_websocket: false,
                    ws_frames: Vec::new(),
                    is_mock: false,
                    timestamp: chrono::Local::now(),
                }))
                .await;
            Ok(resp.body(Full::new(fwd.body)).unwrap())
        }
        Err(_) => {
            let duration = start.elapsed();
            let _ = tx
                .send(TunnelEvent::HttpRequest(HttpRequest {
                    method,
                    path,
                    status: 502,
                    duration,
                    remote_ip,
                    country,
                    user_agent,
                    request_headers,
                    response_headers: Vec::new(),
                    request_size: 0,
                    response_size: 0,
                    request_body: None,
                    response_body: None,
                    is_websocket: false,
                    ws_frames: Vec::new(),
                    is_mock: false,
                    timestamp: chrono::Local::now(),
                }))
                .await;
            Ok(Response::builder()
                .status(hyper::StatusCode::BAD_GATEWAY)
                .body(Full::new(Bytes::from("Bad Gateway")))
                .unwrap())
        }
    }
}

/// Max body size we'll buffer in memory (request or response). 50 MB.
const MAX_BODY_SIZE: usize = 50 * 1024 * 1024;

/// Max body content we store for the detail view (32 KB).
const MAX_BODY_PREVIEW: usize = 32 * 1024;

struct ForwardResult {
    status: hyper::StatusCode,
    headers: hyper::HeaderMap,
    body: Bytes,
    request_size: u64,
    request_body_preview: Option<String>,
}

async fn forward(
    req: Request<Incoming>,
    target_uri: &str,
) -> std::result::Result<ForwardResult, Box<dyn std::error::Error + Send + Sync>> {
    let method = req.method().clone();
    let mut headers = req.headers().clone();
    headers.remove(hyper::header::HOST);

    let body_bytes = Limited::new(req, MAX_BODY_SIZE)
        .collect()
        .await
        .map(|c| c.to_bytes())
        .unwrap_or_default();
    let request_size = body_bytes.len() as u64;
    let request_body_preview = body_preview(&body_bytes);

    let client = reqwest::Client::new();
    let resp = client
        .request(method, target_uri)
        .headers(reqwest_headers(&headers))
        .body(body_bytes)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await?;

    let status = resp.status();
    let resp_headers = resp.headers().clone();
    // Response body is forwarded to the client and only a capped preview
    // (MAX_BODY_PREVIEW) is stored in memory via body_preview().
    // Limit download size to MAX_BODY_SIZE to prevent unbounded memory use.
    let resp_body = {
        let full = resp.bytes().await.unwrap_or_default();
        if full.len() > MAX_BODY_SIZE {
            full.slice(..MAX_BODY_SIZE)
        } else {
            full
        }
    };

    Ok(ForwardResult {
        status,
        headers: reqwest_to_hyper_headers(&resp_headers),
        body: resp_body,
        request_size,
        request_body_preview,
    })
}

fn header_val(headers: &hyper::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

fn collect_headers(headers: &hyper::HeaderMap) -> Vec<HeaderPair> {
    const MAX_HEADER_VALUE_LEN: usize = 2048;

    headers
        .iter()
        .map(|(k, v)| {
            let val = v.to_str().unwrap_or("<binary>");
            HeaderPair {
                name: k.as_str().to_string(),
                value: if val.len() > MAX_HEADER_VALUE_LEN {
                    format!("{}...", &val[..MAX_HEADER_VALUE_LEN])
                } else {
                    val.to_string()
                },
            }
        })
        .collect()
}

/// Try to produce a UTF-8 preview of the body. Returns None for empty or
/// binary bodies. Truncates at MAX_BODY_PREVIEW bytes. Pretty-prints JSON.
fn body_preview(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    // Always try to pretty-print JSON on the full body first
    let pretty = std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|json| serde_json::to_string_pretty(&json).ok());

    if let Some(formatted) = pretty {
        if formatted.len() > MAX_BODY_PREVIEW {
            let truncated = &formatted[..MAX_BODY_PREVIEW];
            // Cut at last newline for clean break
            let cut = truncated.rfind('\n').unwrap_or(MAX_BODY_PREVIEW);
            Some(format!("{}\n\n[truncated — {} total]", &formatted[..cut], format_body_size(bytes.len())))
        } else {
            Some(formatted)
        }
    } else {
        // Not JSON — show raw text or binary
        let slice = if bytes.len() > MAX_BODY_PREVIEW {
            &bytes[..MAX_BODY_PREVIEW]
        } else {
            bytes
        };
        match std::str::from_utf8(slice) {
            Ok(s) => {
                if bytes.len() > MAX_BODY_PREVIEW {
                    Some(format!("{}...\n\n[truncated — {} total]", s, format_body_size(bytes.len())))
                } else {
                    Some(s.to_string())
                }
            }
            Err(_) => Some(format!("[binary data — {}]", format_body_size(bytes.len()))),
        }
    }
}

fn format_body_size(n: usize) -> String {
    if n < 1024 {
        format!("{} B", n)
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

fn reqwest_headers(headers: &hyper::HeaderMap) -> reqwest::header::HeaderMap {
    let mut out = reqwest::header::HeaderMap::new();
    for (key, val) in headers.iter() {
        if let (Ok(k), Ok(v)) = (
            reqwest::header::HeaderName::from_bytes(key.as_str().as_bytes()),
            reqwest::header::HeaderValue::from_bytes(val.as_bytes()),
        ) {
            out.insert(k, v);
        }
    }
    out
}

fn reqwest_to_hyper_headers(headers: &reqwest::header::HeaderMap) -> hyper::HeaderMap {
    let mut out = hyper::HeaderMap::new();
    for (key, val) in headers.iter() {
        if let (Ok(k), Ok(v)) = (
            hyper::header::HeaderName::from_bytes(key.as_str().as_bytes()),
            hyper::header::HeaderValue::from_bytes(val.as_bytes()),
        ) {
            out.insert(k, v);
        }
    }
    out
}

/// Max payload preview size for WebSocket text frames (4 KB).
const MAX_WS_PAYLOAD_PREVIEW: usize = 4 * 1024;

fn is_websocket_upgrade(headers: &hyper::HeaderMap) -> bool {
    let has_upgrade_connection = headers
        .get(hyper::header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_lowercase().contains("upgrade"))
        .unwrap_or(false);

    let has_websocket_upgrade = headers
        .get(hyper::header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);

    has_upgrade_connection && has_websocket_upgrade
}

#[allow(clippy::too_many_arguments)]
async fn handle_websocket(
    req: Request<Incoming>,
    target_port: u16,
    path: &str,
    method: &str,
    start: Instant,
    remote_ip: Option<String>,
    country: Option<String>,
    user_agent: Option<String>,
    request_headers: Vec<HeaderPair>,
    tx: mpsc::Sender<TunnelEvent>,
) -> std::result::Result<Response<Full<Bytes>>, hyper::Error> {
    let target_ws_uri = format!("ws://127.0.0.1:{}{}", target_port, path);

    // Connect to the target WebSocket server
    let target_conn = tokio_tungstenite::connect_async(&target_ws_uri).await;
    let (target_ws, _) = match target_conn {
        Ok(conn) => conn,
        Err(e) => {
            tracing::debug!("websocket target connection failed: {}", e);
            let duration = start.elapsed();
            let _ = tx
                .send(TunnelEvent::HttpRequest(HttpRequest {
                    method: method.to_string(),
                    path: path.to_string(),
                    status: 502,
                    duration,
                    remote_ip,
                    country,
                    user_agent,
                    request_headers,
                    response_headers: Vec::new(),
                    request_size: 0,
                    response_size: 0,
                    request_body: None,
                    response_body: None,
                    is_websocket: true,
                    ws_frames: Vec::new(),
                    is_mock: false,
                    timestamp: chrono::Local::now(),
                }))
                .await;
            return Ok(Response::builder()
                .status(hyper::StatusCode::BAD_GATEWAY)
                .body(Full::new(Bytes::from("WebSocket Bad Gateway")))
                .unwrap());
        }
    };

    // Send the initial upgrade HttpRequest event
    let duration = start.elapsed();
    let req_id_path = path.to_string();
    let req_id_method = method.to_string();
    let _ = tx
        .send(TunnelEvent::HttpRequest(HttpRequest {
            method: req_id_method,
            path: req_id_path.clone(),
            status: 101,
            duration,
            remote_ip,
            country,
            user_agent,
            request_headers,
            response_headers: Vec::new(),
            request_size: 0,
            response_size: 0,
            request_body: None,
            response_body: None,
            is_websocket: true,
            ws_frames: Vec::new(),
            is_mock: false,
            timestamp: chrono::Local::now(),
        }))
        .await;

    // Upgrade the client connection
    let upgraded = match hyper::upgrade::on(req).await {
        Ok(upgraded) => upgraded,
        Err(e) => {
            tracing::debug!("client websocket upgrade failed: {}", e);
            return Ok(Response::builder()
                .status(hyper::StatusCode::INTERNAL_SERVER_ERROR)
                .body(Full::new(Bytes::from("Upgrade failed")))
                .unwrap());
        }
    };

    let client_ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
        TokioIo::new(upgraded),
        tungstenite::protocol::Role::Server,
        None,
    )
    .await;

    // Bridge the two WebSocket connections
    let (mut client_sink, mut client_stream) = client_ws.split();
    let (mut target_sink, mut target_stream) = target_ws.split();
    let tx_c2s = tx.clone();
    let tx_s2c = tx.clone();

    // Client -> Server
    let c2s = tokio::spawn(async move {
        while let Some(msg) = client_stream.next().await {
            match msg {
                Ok(msg) => {
                    let frame = tungstenite_msg_to_frame(&msg, WsDirection::ClientToServer);
                    let _ = tx_c2s.send(TunnelEvent::WebSocketFrame(frame)).await;
                    if msg.is_close() {
                        let _ = target_sink.close().await;
                        break;
                    }
                    if target_sink.send(msg).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Server -> Client
    let s2c = tokio::spawn(async move {
        while let Some(msg) = target_stream.next().await {
            match msg {
                Ok(msg) => {
                    let frame = tungstenite_msg_to_frame(&msg, WsDirection::ServerToClient);
                    let _ = tx_s2c.send(TunnelEvent::WebSocketFrame(frame)).await;
                    if msg.is_close() {
                        let _ = client_sink.close().await;
                        break;
                    }
                    if client_sink.send(msg).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Wait for either direction to finish, then abort the other
    tokio::select! {
        _ = c2s => {},
        _ = s2c => {},
    }

    // Return the 101 Switching Protocols response
    Ok(Response::builder()
        .status(hyper::StatusCode::SWITCHING_PROTOCOLS)
        .header(hyper::header::CONNECTION, "Upgrade")
        .header(hyper::header::UPGRADE, "websocket")
        .body(Full::new(Bytes::new()))
        .unwrap())
}

fn tungstenite_msg_to_frame(
    msg: &tungstenite::Message,
    direction: WsDirection,
) -> WebSocketFrame {
    let (opcode, payload_preview, payload_size) = match msg {
        tungstenite::Message::Text(text) => {
            let preview = if text.len() > MAX_WS_PAYLOAD_PREVIEW {
                Some(text[..MAX_WS_PAYLOAD_PREVIEW].to_string())
            } else {
                Some(text.to_string())
            };
            (WsOpcode::Text, preview, text.len() as u64)
        }
        tungstenite::Message::Binary(data) => {
            (WsOpcode::Binary, None, data.len() as u64)
        }
        tungstenite::Message::Ping(data) => {
            (WsOpcode::Ping, None, data.len() as u64)
        }
        tungstenite::Message::Pong(data) => {
            (WsOpcode::Pong, None, data.len() as u64)
        }
        tungstenite::Message::Close(_) => {
            (WsOpcode::Close, None, 0)
        }
        tungstenite::Message::Frame(_) => {
            (WsOpcode::Binary, None, 0)
        }
    };

    WebSocketFrame {
        direction,
        opcode,
        payload_preview,
        payload_size,
        timestamp: Instant::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_websocket_upgrade() {
        let mut headers = hyper::HeaderMap::new();
        headers.insert(hyper::header::CONNECTION, "Upgrade".parse().unwrap());
        headers.insert(hyper::header::UPGRADE, "websocket".parse().unwrap());
        assert!(is_websocket_upgrade(&headers));
    }

    #[test]
    fn detect_websocket_upgrade_case_insensitive() {
        let mut headers = hyper::HeaderMap::new();
        headers.insert(hyper::header::CONNECTION, "upgrade".parse().unwrap());
        headers.insert(hyper::header::UPGRADE, "WebSocket".parse().unwrap());
        assert!(is_websocket_upgrade(&headers));
    }

    #[test]
    fn no_websocket_without_upgrade_header() {
        let mut headers = hyper::HeaderMap::new();
        headers.insert(hyper::header::CONNECTION, "keep-alive".parse().unwrap());
        assert!(!is_websocket_upgrade(&headers));
    }

    #[test]
    fn no_websocket_with_partial_headers() {
        let mut headers = hyper::HeaderMap::new();
        headers.insert(hyper::header::CONNECTION, "Upgrade".parse().unwrap());
        // Missing Upgrade: websocket header
        assert!(!is_websocket_upgrade(&headers));
    }

    #[test]
    fn no_websocket_empty_headers() {
        let headers = hyper::HeaderMap::new();
        assert!(!is_websocket_upgrade(&headers));
    }

    #[test]
    fn tungstenite_text_frame_conversion() {
        let msg = tungstenite::Message::Text("hello".into());
        let frame = tungstenite_msg_to_frame(&msg, WsDirection::ClientToServer);
        assert!(matches!(frame.opcode, WsOpcode::Text));
        assert_eq!(frame.payload_preview.as_deref(), Some("hello"));
        assert_eq!(frame.payload_size, 5);
    }

    #[test]
    fn tungstenite_binary_frame_conversion() {
        let msg = tungstenite::Message::Binary(vec![1, 2, 3].into());
        let frame = tungstenite_msg_to_frame(&msg, WsDirection::ServerToClient);
        assert!(matches!(frame.opcode, WsOpcode::Binary));
        assert!(frame.payload_preview.is_none());
        assert_eq!(frame.payload_size, 3);
    }

    #[test]
    fn tungstenite_text_frame_truncation() {
        let long_text = "x".repeat(MAX_WS_PAYLOAD_PREVIEW + 100);
        let msg = tungstenite::Message::Text(long_text.clone().into());
        let frame = tungstenite_msg_to_frame(&msg, WsDirection::ClientToServer);
        assert!(matches!(frame.opcode, WsOpcode::Text));
        assert_eq!(frame.payload_preview.as_ref().unwrap().len(), MAX_WS_PAYLOAD_PREVIEW);
        assert_eq!(frame.payload_size, long_text.len() as u64);
    }
}
