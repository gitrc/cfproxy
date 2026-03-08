use std::path::Path;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::error::{Error, Result};
use crate::event::TunnelEvent;

pub struct Tunnel {
    child: Option<tokio::process::Child>,
}

impl Tunnel {
    pub fn new() -> Self {
        Self { child: None }
    }

    /// Start cloudflared and stream parsed events through `tx`.
    /// This method blocks until the process exits.
    ///
    /// If `token` is provided, uses a named tunnel (`tunnel run --token`).
    /// Otherwise, creates a quick tunnel (`tunnel --url`).
    ///
    /// If `custom_url` is provided, emits it as a URL event when the first
    /// edge connection is registered (used for API-provisioned tunnels).
    pub async fn start(
        &mut self,
        binary: &Path,
        port: u16,
        token: Option<&str>,
        custom_url: Option<String>,
        tx: mpsc::Sender<TunnelEvent>,
    ) -> Result<()> {
        let _ = tx.send(TunnelEvent::Connecting).await;

        let mut cmd = Command::new(binary);
        if let Some(token) = token {
            cmd.args(["tunnel", "run", "--token", token]);
        } else {
            cmd.args(["tunnel", "--url", &format!("http://localhost:{}", port)]);
        }
        let mut child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| Error::Tunnel("failed to capture stderr".into()))?;

        self.child = Some(child);

        let mut reader = BufReader::new(stderr).lines();
        let mut url_sent = false;
        while let Some(line) = reader.next_line().await? {
            if let Some(event) = parse_line(&line) {
                // For API-provisioned tunnels, emit the custom URL when the
                // first edge connection registers (EdgeLocation event).
                if !url_sent {
                    if let Some(ref url) = custom_url {
                        if matches!(&event, TunnelEvent::EdgeLocation(_)) {
                            url_sent = true;
                            let _ = tx.send(TunnelEvent::Url(url.clone())).await;
                        }
                    }
                }
                if tx.send(event).await.is_err() {
                    break;
                }
            }
        }

        let status = if let Some(ref mut c) = self.child {
            c.wait().await?
        } else {
            return Ok(());
        };

        let msg = format!("cloudflared exited with {}", status);
        let _ = tx.send(TunnelEvent::Disconnected(msg)).await;
        Ok(())
    }
}

/// Parse a single line of cloudflared output into an event.
pub fn parse_line(line: &str) -> Option<TunnelEvent> {
    // URL pattern: https://<subdomain>.trycloudflare.com
    if let Some(start) = line.find("https://") {
        let url_part = &line[start..];
        if let Some(end) = url_part.find(|c: char| c.is_whitespace()) {
            let url = &url_part[..end];
            if url.contains(".trycloudflare.com") {
                return Some(TunnelEvent::Url(url.to_string()));
            }
        } else if url_part.contains(".trycloudflare.com") {
            return Some(TunnelEvent::Url(url_part.to_string()));
        }
    }

    // Metrics port: "Starting metrics server on 127.0.0.1:<port>"
    if let Some(idx) = line.find("Starting metrics server on 127.0.0.1:") {
        let after = &line[idx + "Starting metrics server on 127.0.0.1:".len()..];
        let port_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(port) = port_str.parse::<u16>() {
            return Some(TunnelEvent::MetricsPort(port));
        }
    }

    // Edge location: "location=XXX"
    if let Some(idx) = line.find("location=") {
        let after = &line[idx + "location=".len()..];
        let loc: String = after
            .chars()
            .take_while(|c| c.is_alphanumeric())
            .collect();
        if !loc.is_empty() {
            return Some(TunnelEvent::EdgeLocation(loc));
        }
    }

    // Version: "Version X.Y.Z"
    if let Some(idx) = line.find("Version ") {
        let after = &line[idx + "Version ".len()..];
        let ver: String = after
            .chars()
            .take_while(|c| *c == '.' || c.is_ascii_digit())
            .collect();
        if !ver.is_empty() {
            return Some(TunnelEvent::Version(ver));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_url() {
        let line = "2024-01-01 INF +-----------------------------------------------------------+";
        assert!(parse_line(line).is_none());

        let line = "2024-01-01 INF |  https://foo-bar-baz.trycloudflare.com                   |";
        match parse_line(line) {
            Some(TunnelEvent::Url(url)) => {
                assert_eq!(url, "https://foo-bar-baz.trycloudflare.com");
            }
            other => panic!("expected Url event, got {:?}", other),
        }
    }

    #[test]
    fn parse_url_end_of_line() {
        let line = "https://test-tunnel.trycloudflare.com";
        match parse_line(line) {
            Some(TunnelEvent::Url(url)) => {
                assert_eq!(url, "https://test-tunnel.trycloudflare.com");
            }
            other => panic!("expected Url, got {:?}", other),
        }
    }

    #[test]
    fn parse_metrics_port() {
        let line = "2024-01-01 INF Starting metrics server on 127.0.0.1:43567/metrics";
        match parse_line(line) {
            Some(TunnelEvent::MetricsPort(port)) => assert_eq!(port, 43567),
            other => panic!("expected MetricsPort, got {:?}", other),
        }
    }

    #[test]
    fn parse_edge_location() {
        let line = "2024-01-01 INF Connection registered connIndex=0 location=lax";
        match parse_line(line) {
            Some(TunnelEvent::EdgeLocation(loc)) => assert_eq!(loc, "lax"),
            other => panic!("expected EdgeLocation, got {:?}", other),
        }
    }

    #[test]
    fn parse_version() {
        let line = "2024-01-01 INF Version 2024.1.5";
        match parse_line(line) {
            Some(TunnelEvent::Version(v)) => assert_eq!(v, "2024.1.5"),
            other => panic!("expected Version, got {:?}", other),
        }
    }

    #[test]
    fn parse_irrelevant_line() {
        assert!(parse_line("some random log line").is_none());
        assert!(parse_line("").is_none());
    }
}
