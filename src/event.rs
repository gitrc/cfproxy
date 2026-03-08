use std::time::{Duration, Instant};

use chrono::{DateTime, Local};

use crate::metrics::TunnelMetrics;

#[derive(Debug, Clone)]
pub enum WsDirection {
    ClientToServer,
    ServerToClient,
}

#[derive(Debug, Clone)]
pub enum WsOpcode {
    Text,
    Binary,
    Ping,
    Pong,
    Close,
}

#[derive(Debug, Clone)]
pub struct WebSocketFrame {
    pub direction: WsDirection,
    pub opcode: WsOpcode,
    pub payload_preview: Option<String>,
    pub payload_size: u64,
    pub timestamp: Instant,
}

#[derive(Debug, Clone)]
pub struct HeaderPair {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub status: u16,
    pub duration: Duration,
    pub remote_ip: Option<String>,
    pub country: Option<String>,
    pub user_agent: Option<String>,
    pub request_headers: Vec<HeaderPair>,
    pub response_headers: Vec<HeaderPair>,
    pub request_size: u64,
    pub response_size: u64,
    pub request_body: Option<String>,
    pub response_body: Option<String>,
    pub is_websocket: bool,
    pub ws_frames: Vec<WebSocketFrame>,
    pub is_mock: bool,
    pub timestamp: DateTime<Local>,
}

impl HttpRequest {
    /// Build a cURL command that reproduces this request against the given base URL.
    pub fn to_curl(&self, base_url: &str) -> String {
        let mut parts = vec![format!("curl -X {}", self.method)];
        // Add headers (skip cloudflare internal headers)
        for h in &self.request_headers {
            let lower = h.name.to_lowercase();
            if lower.starts_with("cf-") || lower == "cdn-loop" || lower == "x-forwarded-for"
                || lower == "x-forwarded-proto" || lower == "host"
            {
                continue;
            }
            parts.push(format!("  -H '{}: {}'", h.name, h.value.replace('\'', "'\\''")));
        }
        // Add body
        if let Some(body) = &self.request_body {
            let escaped = body.replace('\'', "'\\''");
            if escaped.len() <= 2048 {
                parts.push(format!("  -d '{}'", escaped));
            } else {
                parts.push(format!("  -d '{}'", &escaped[..2048]));
            }
        }
        parts.push(format!("  '{}{}'", base_url.trim_end_matches('/'), self.path));
        parts.join(" \\\n")
    }
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum TunnelEvent {
    Connecting,
    Url(String),
    EdgeLocation(String),
    Version(String),
    MetricsPort(u16),
    Metrics(TunnelMetrics),
    HttpRequest(HttpRequest),
    WebSocketFrame(WebSocketFrame),
    Disconnected(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_frame_creation() {
        let frame = WebSocketFrame {
            direction: WsDirection::ClientToServer,
            opcode: WsOpcode::Text,
            payload_preview: Some("{\"event\":\"ping\"}".into()),
            payload_size: 16,
            timestamp: Instant::now(),
        };
        assert_eq!(frame.payload_size, 16);
        assert!(frame.payload_preview.is_some());
        assert!(matches!(frame.direction, WsDirection::ClientToServer));
        assert!(matches!(frame.opcode, WsOpcode::Text));
    }

    #[test]
    fn websocket_frame_binary() {
        let frame = WebSocketFrame {
            direction: WsDirection::ServerToClient,
            opcode: WsOpcode::Binary,
            payload_preview: None,
            payload_size: 2048,
            timestamp: Instant::now(),
        };
        assert_eq!(frame.payload_size, 2048);
        assert!(frame.payload_preview.is_none());
        assert!(matches!(frame.direction, WsDirection::ServerToClient));
        assert!(matches!(frame.opcode, WsOpcode::Binary));
    }

    #[test]
    fn websocket_frame_control() {
        let ping = WebSocketFrame {
            direction: WsDirection::ClientToServer,
            opcode: WsOpcode::Ping,
            payload_preview: None,
            payload_size: 0,
            timestamp: Instant::now(),
        };
        assert!(matches!(ping.opcode, WsOpcode::Ping));

        let pong = WebSocketFrame {
            direction: WsDirection::ServerToClient,
            opcode: WsOpcode::Pong,
            payload_preview: None,
            payload_size: 0,
            timestamp: Instant::now(),
        };
        assert!(matches!(pong.opcode, WsOpcode::Pong));

        let close = WebSocketFrame {
            direction: WsDirection::ClientToServer,
            opcode: WsOpcode::Close,
            payload_preview: None,
            payload_size: 0,
            timestamp: Instant::now(),
        };
        assert!(matches!(close.opcode, WsOpcode::Close));
    }

    #[test]
    fn http_request_websocket_defaults() {
        let req = HttpRequest {
            method: "GET".into(),
            path: "/ws".into(),
            status: 101,
            duration: Duration::from_millis(0),
            remote_ip: None,
            country: None,
            user_agent: None,
            request_headers: Vec::new(),
            response_headers: Vec::new(),
            request_size: 0,
            response_size: 0,
            request_body: None,
            response_body: None,
            is_websocket: false,
            ws_frames: Vec::new(),
            is_mock: false,
            timestamp: Local::now(),
        };
        assert!(!req.is_websocket);
        assert!(req.ws_frames.is_empty());
        assert!(!req.is_mock);
    }
}
