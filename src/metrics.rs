use crate::error::{Error, Result};

#[derive(Debug, Clone, Default)]
pub struct TunnelMetrics {
    pub total_requests: u64,
    pub request_errors: u64,
}

/// Fetch metrics from cloudflared's prometheus endpoint.
pub async fn fetch(port: u16) -> Result<TunnelMetrics> {
    let url = format!("http://127.0.0.1:{}/metrics", port);
    let body = reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(1))
        .send()
        .await
        .map_err(|e| Error::Metrics(e.to_string()))?
        .text()
        .await
        .map_err(|e| Error::Metrics(e.to_string()))?;
    Ok(parse(&body))
}

/// Parse prometheus text format into TunnelMetrics.
pub fn parse(body: &str) -> TunnelMetrics {
    let mut metrics = TunnelMetrics::default();
    for line in body.lines() {
        if let Some(val) = line.strip_prefix("cloudflared_tunnel_total_requests ") {
            metrics.total_requests = parse_metric_value(val);
        } else if let Some(val) = line.strip_prefix("cloudflared_tunnel_request_errors ") {
            metrics.request_errors = parse_metric_value(val);
        }
    }
    metrics
}

fn parse_metric_value(s: &str) -> u64 {
    s.trim().parse::<f64>().unwrap_or(0.0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_body() {
        let m = parse("");
        assert_eq!(m.total_requests, 0);
        assert_eq!(m.request_errors, 0);
    }

    #[test]
    fn parse_valid_metrics() {
        let body = "\
# HELP cloudflared_tunnel_total_requests Total requests
# TYPE cloudflared_tunnel_total_requests counter
cloudflared_tunnel_total_requests 42
# HELP cloudflared_tunnel_request_errors Total request errors
# TYPE cloudflared_tunnel_request_errors counter
cloudflared_tunnel_request_errors 3
";
        let m = parse(body);
        assert_eq!(m.total_requests, 42);
        assert_eq!(m.request_errors, 3);
    }

    #[test]
    fn parse_float_values() {
        let body = "cloudflared_tunnel_total_requests 123.0\n";
        let m = parse(body);
        assert_eq!(m.total_requests, 123);
    }

    #[test]
    fn parse_ignores_unrelated_lines() {
        let body = "\
some_other_metric 999
cloudflared_tunnel_total_requests 5
another_metric 100
";
        let m = parse(body);
        assert_eq!(m.total_requests, 5);
        assert_eq!(m.request_errors, 0);
    }
}
