use std::time::Duration;

use crate::event::HttpRequest;

pub fn copy_to_clipboard(text: &str) -> bool {
    use std::process::{Command, Stdio};
    if let Ok(mut child) = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(text.as_bytes());
        }
        return child.wait().map(|s| s.success()).unwrap_or(false);
    }
    if let Ok(mut child) = Command::new("xclip")
        .args(["-selection", "clipboard"])
        .stdin(Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(text.as_bytes());
        }
        return child.wait().map(|s| s.success()).unwrap_or(false);
    }
    false
}

pub fn build_replay_request(
    req: &HttpRequest,
    base_url: &str,
) -> (String, String, Vec<(String, String)>, Option<String>) {
    let url = format!("{}{}", base_url.trim_end_matches('/'), req.path);
    let method = req.method.clone();
    let headers: Vec<(String, String)> = req
        .request_headers
        .iter()
        .filter(|h| {
            let lower = h.name.to_lowercase();
            !lower.starts_with("cf-")
                && lower != "cdn-loop"
                && lower != "x-forwarded-for"
                && lower != "x-forwarded-proto"
                && lower != "host"
        })
        .map(|h| (h.name.clone(), h.value.clone()))
        .collect();
    let body = req.request_body.clone();
    (method, url, headers, body)
}

fn spawn_replay(method: String, url: String, headers: Vec<(String, String)>, body: Option<String>) {
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let method = method.parse::<reqwest::Method>().unwrap_or(reqwest::Method::GET);
        let mut builder = client.request(method, &url);
        for (k, v) in &headers {
            builder = builder.header(k.as_str(), v.as_str());
        }
        if let Some(b) = body {
            builder = builder.body(b);
        }
        let _ = builder.timeout(std::time::Duration::from_secs(30)).send().await;
    });
}

pub fn replay_request(req: &HttpRequest, port: u16) {
    let base_url = format!("http://127.0.0.1:{}", port);
    let (method, url, headers, body) = build_replay_request(req, &base_url);
    spawn_replay(method, url, headers, body);
}

pub fn replay_to_url(req: &HttpRequest, base_url: &str) {
    let (method, url, headers, body) = build_replay_request(req, base_url);
    spawn_replay(method, url, headers, body);
}

pub fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

pub fn format_request_duration(d: Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        format!("{}ms", ms)
    } else {
        format!("{:.1}s", d.as_secs_f64())
    }
}

pub fn format_bytes(b: u64) -> String {
    if b == 0 {
        "-".to_string()
    } else if b < 1024 {
        format!("{} B", b)
    } else if b < 1024 * 1024 {
        format!("{:.1} KB", b as f64 / 1024.0)
    } else {
        format!("{:.1} MB", b as f64 / (1024.0 * 1024.0))
    }
}

/// Extract OS/platform from user-agent string.
pub fn parse_ua_platform(ua: &str) -> String {
    let ua_lower = ua.to_lowercase();

    // Mobile first
    if ua_lower.contains("iphone") { return "iPhone".into(); }
    if ua_lower.contains("ipad") { return "iPad".into(); }
    if ua_lower.contains("android") {
        // Try to get device info
        if let Some(start) = ua.find("Android") {
            if let Some(semi) = ua[start..].find(';') {
                let after = &ua[start + semi + 1..];
                if let Some(end) = after.find(')').or_else(|| after.find(';')) {
                    let device = after[..end].trim();
                    if !device.is_empty() && device.len() <= 20 {
                        return device.to_string();
                    }
                }
            }
        }
        return "Android".into();
    }

    // Desktop OS
    if ua_lower.contains("macintosh") || ua_lower.contains("mac os") { return "macOS".into(); }
    if ua_lower.contains("windows") { return "Windows".into(); }
    if ua_lower.contains("linux") && !ua_lower.contains("android") { return "Linux".into(); }
    if ua_lower.contains("cros") { return "ChromeOS".into(); }

    // CLI/SDK tools don't have a platform
    if ua_lower.starts_with("curl/") || ua_lower.starts_with("wget/")
        || ua_lower.starts_with("httpie/") || ua_lower.contains("postman")
        || ua_lower.contains("insomnia") || ua_lower.starts_with("python")
        || ua_lower.starts_with("go-http") || ua_lower.starts_with("java")
        || ua_lower.contains("okhttp") || ua_lower.starts_with("ruby")
        || ua_lower.starts_with("rust") || ua_lower.contains("reqwest")
        || ua_lower.contains("node-fetch") || ua_lower.contains("undici")
        || ua_lower.starts_with("dart")
    {
        return "-".into();
    }

    "-".into()
}

pub fn parse_ua(ua: &str) -> String {
    let ua_lower = ua.to_lowercase();

    // Bots and crawlers
    if ua_lower.contains("googlebot") { return "Googlebot".into(); }
    if ua_lower.contains("bingbot") { return "Bingbot".into(); }
    if ua_lower.contains("slurp") { return "Yahoo Bot".into(); }
    if ua_lower.contains("duckduckbot") { return "DuckDuckBot".into(); }
    if ua_lower.contains("baiduspider") { return "Baidu".into(); }
    if ua_lower.contains("yandexbot") { return "YandexBot".into(); }
    if ua_lower.contains("facebookexternalhit") { return "Facebook".into(); }
    if ua_lower.contains("twitterbot") { return "Twitter".into(); }
    if ua_lower.contains("linkedinbot") { return "LinkedIn".into(); }
    if ua_lower.contains("slackbot") { return "Slack".into(); }
    if ua_lower.contains("telegrambot") { return "Telegram".into(); }
    if ua_lower.contains("discordbot") { return "Discord".into(); }
    if ua_lower.contains("whatsapp") { return "WhatsApp".into(); }

    // CLI tools
    if ua_lower.starts_with("curl/") { return extract_tool_version(ua, "curl"); }
    if ua_lower.starts_with("wget/") { return extract_tool_version(ua, "wget"); }
    if ua_lower.starts_with("httpie/") { return extract_tool_version(ua, "HTTPie"); }
    if ua_lower.contains("postman") { return "Postman".into(); }
    if ua_lower.contains("insomnia") { return "Insomnia".into(); }
    if ua_lower.starts_with("python-requests") { return "python-requests".into(); }
    if ua_lower.starts_with("python-urllib") { return "Python".into(); }
    if ua_lower.starts_with("go-http-client") { return "Go".into(); }
    if ua_lower.contains("node-fetch") || ua_lower.contains("undici") { return "Node.js".into(); }
    if ua_lower.starts_with("java/") || ua_lower.contains("okhttp") { return "Java".into(); }
    if ua_lower.starts_with("ruby") { return "Ruby".into(); }
    if ua_lower.starts_with("rust") || ua_lower.contains("reqwest") { return "Rust".into(); }
    if ua_lower.starts_with("dart") { return "Dart".into(); }

    // Browsers — order matters (check specific before generic)
    if let Some(v) = extract_browser_version(ua, "Edg/") { return format!("Edge {v}"); }
    if let Some(v) = extract_browser_version(ua, "OPR/") { return format!("Opera {v}"); }
    if let Some(v) = extract_browser_version(ua, "Brave/") { return format!("Brave {v}"); }
    if let Some(v) = extract_browser_version(ua, "Vivaldi/") { return format!("Vivaldi {v}"); }
    if ua.contains("Chrome/") && !ua.contains("Edg/") && !ua.contains("OPR/") {
        if let Some(v) = extract_browser_version(ua, "Chrome/") { return format!("Chrome {v}"); }
    }
    if ua.contains("Safari/") && !ua.contains("Chrome/") && !ua.contains("Chromium/") {
        if let Some(v) = extract_browser_version(ua, "Version/") { return format!("Safari {v}"); }
        return "Safari".into();
    }
    if let Some(v) = extract_browser_version(ua, "Firefox/") { return format!("Firefox {v}"); }

    // Mobile
    if ua_lower.contains("cfnetwork") { return "iOS App".into(); }
    if ua_lower.contains("dalvik") { return "Android App".into(); }

    // Fallback: first token, truncated
    let first = ua.split_whitespace().next().unwrap_or(ua);
    if first.len() > 20 {
        format!("{}...", &first[..17])
    } else {
        first.to_string()
    }
}

fn extract_browser_version(ua: &str, token: &str) -> Option<String> {
    let start = ua.find(token)? + token.len();
    let rest = &ua[start..];
    let end = rest.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(rest.len());
    let version = &rest[..end];
    // Return just major version
    Some(version.split('.').next().unwrap_or(version).to_string())
}

fn extract_tool_version(ua: &str, name: &str) -> String {
    let version = ua.split('/').nth(1).unwrap_or("");
    let major = version.split('.').take(2).collect::<Vec<_>>().join(".");
    if major.is_empty() { name.to_string() } else { format!("{name}/{major}") }
}

pub fn truncate_header_value(v: &str) -> String {
    if v.len() > 120 {
        format!("{}...", &v[..117])
    } else {
        v.to_string()
    }
}
