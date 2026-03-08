use ratatui::prelude::*;
use ratatui::widgets::{Cell, Row, Table};

use super::helpers::{format_request_duration, parse_ua, parse_ua_platform};
use super::state::AppState;

pub fn render(f: &mut Frame, state: &AppState, area: Rect) {
    let header = Row::new(vec![
        "Time", "Method", "Path", "Status", "Size", "Type", "Duration", "IP", "User-Agent",
    ])
    .style(Style::default().fg(Color::DarkGray).bold())
    .bottom_margin(0);

    let dim = Style::default().fg(Color::DarkGray);
    let visible = area.height.saturating_sub(1) as usize;

    let filtered: Vec<&crate::event::HttpRequest> = if state.filter.is_empty() {
        state.requests.iter().rev().collect()
    } else {
        let f_lower = state.filter.to_lowercase();
        state.requests.iter().rev().filter(|req| {
            req.path.to_lowercase().contains(&f_lower)
                || req.method.to_lowercase().contains(&f_lower)
                || req.status.to_string().contains(&f_lower)
                || req.user_agent.as_deref().unwrap_or("").to_lowercase().contains(&f_lower)
                || req.remote_ip.as_deref().unwrap_or("").contains(&f_lower)
                || short_content_type(&req.response_headers).to_lowercase().contains(&f_lower)
        }).collect()
    };

    let scroll_offset = if let Some(sel) = state.selected {
        if sel >= visible { sel - visible + 1 } else { 0 }
    } else {
        0
    };

    let rows: Vec<Row> = filtered
        .iter()
        .skip(scroll_offset)
        .take(visible)
        .enumerate()
        .map(|(display_idx, req)| {
            let is_selected = state.selected.is_some_and(|sel| display_idx == sel - scroll_offset);
            let status_style = match req.status {
                200..=299 => Style::default().fg(Color::Green),
                300..=399 => Style::default().fg(Color::Cyan),
                400..=499 => Style::default().fg(Color::Yellow),
                _ => Style::default().fg(Color::Red),
            };
            let method_style = Style::default().fg(Color::Rgb(0, 200, 255)).bold();
            let ip = req.remote_ip.as_deref().unwrap_or("-");
            let raw_ua = req.user_agent.as_deref().unwrap_or("-");
            let browser = parse_ua(raw_ua);
            let platform = parse_ua_platform(raw_ua);
            let ua = if platform == "-" {
                browser
            } else {
                format!("{} \u{00B7} {}", browser, platform)
            };
            let time = req.timestamp.format("%H:%M:%S").to_string();
            let size = format_size(req.response_size);
            let content_type = short_content_type(&req.response_headers);

            let actual_idx = display_idx + scroll_offset;
            let is_marked = state.marked == Some(actual_idx);

            let method_display = if is_marked && req.is_mock {
                format!("* \u{25CF} {}", req.method)
            } else if is_marked {
                format!("* {}", req.method)
            } else if req.is_mock {
                format!("\u{25CF} {}", req.method)
            } else {
                req.method.clone()
            };

            let row = Row::new(vec![
                Cell::from(Span::styled(time, dim)),
                Cell::from(Span::styled(method_display, method_style)),
                Cell::from(&req.path as &str),
                Cell::from(Span::styled(req.status.to_string(), status_style)),
                Cell::from(Span::styled(size, dim)),
                Cell::from(Span::styled(content_type, dim)),
                Cell::from(format_request_duration(req.duration)),
                Cell::from(Span::styled(ip, dim)),
                Cell::from(Span::styled(ua, dim)),
            ]);

            if is_selected {
                row.style(Style::default().bg(Color::Rgb(30, 30, 60)))
            } else {
                row
            }
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(8),   // Time
            Constraint::Length(7),   // Method
            Constraint::Min(12),    // Path
            Constraint::Length(6),   // Status
            Constraint::Length(7),   // Size
            Constraint::Length(6),   // Type
            Constraint::Length(8),   // Duration
            Constraint::Length(16),  // IP
            Constraint::Min(10),    // User-Agent
        ],
    )
    .header(header);

    f.render_widget(table, area);
}

fn format_size(bytes: u64) -> String {
    if bytes == 0 {
        "-".into()
    } else if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn short_content_type(headers: &[crate::event::HeaderPair]) -> String {
    let ct = headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.as_str())
        .unwrap_or("");

    // Strip charset and parameters
    let mime = ct.split(';').next().unwrap_or("").trim();

    match mime {
        "application/json" => "json",
        "application/xml" | "text/xml" => "xml",
        "text/html" => "html",
        "text/css" => "css",
        "text/plain" => "text",
        "text/javascript" | "application/javascript" | "application/x-javascript" => "js",
        "application/wasm" => "wasm",
        "application/octet-stream" => "bin",
        "application/pdf" => "pdf",
        "application/zip" => "zip",
        "application/gzip" => "gzip",
        "multipart/form-data" => "form",
        "application/x-www-form-urlencoded" => "form",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/svg+xml" => "svg",
        "image/webp" => "webp",
        "image/x-icon" | "image/vnd.microsoft.icon" => "ico",
        "font/woff2" => "woff2",
        "font/woff" => "woff",
        "font/ttf" | "font/otf" => "font",
        "video/mp4" => "mp4",
        "audio/mpeg" => "mp3",
        _ if mime.starts_with("image/") => "img",
        _ if mime.starts_with("video/") => "video",
        _ if mime.starts_with("audio/") => "audio",
        _ if mime.starts_with("text/") => "text",
        _ if mime.is_empty() => "-",
        _ => {
            // Try subtype: "application/vnd.api+json" → "json"
            if let Some(sub) = mime.split('/').nth(1) {
                if sub.ends_with("+json") {
                    return "json".into();
                }
                if sub.ends_with("+xml") {
                    return "xml".into();
                }
                if sub.len() <= 6 {
                    return sub.to_string();
                }
            }
            "-"
        }
    }
    .to_string()
}
