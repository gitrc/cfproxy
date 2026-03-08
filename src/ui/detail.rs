use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use super::helpers::{format_bytes, format_request_duration, truncate_header_value};
use super::state::{AppState, DetailTab};

pub fn render(f: &mut Frame, state: &AppState, area: Rect) {
    let req = match state.selected.and_then(|i| {
        let indices = state.filtered_indices();
        indices.get(i).and_then(|&ri| state.requests.iter().rev().nth(ri))
    }) {
        Some(r) => r,
        None => {
            let p = Paragraph::new("No request selected").alignment(Alignment::Center);
            f.render_widget(p, area);
            return;
        }
    };

    let accent = Color::Rgb(0, 255, 220);
    let label_style = Style::default().fg(Color::DarkGray);
    let value_style = Style::default().fg(Color::White);
    let header_style = Style::default().fg(accent).bold();

    // ── Layout: tab bar + content + footer ──
    let tabs = DetailTab::all(req.is_websocket);
    let outer_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),  // tab bar
            Constraint::Min(3),    // content
            Constraint::Length(1), // footer / flash
        ])
        .split(area);

    let tab_titles: Vec<Span> = tabs
        .iter()
        .enumerate()
        .flat_map(|(i, tab)| {
            let mut spans = Vec::new();
            if i > 0 {
                spans.push(Span::styled("  ", Style::default().fg(Color::DarkGray)));
            }
            if *tab == state.detail_tab {
                spans.push(Span::styled(
                    format!(" {} ", tab.label()),
                    Style::default().fg(Color::White).bg(Color::Rgb(40, 40, 80)).bold(),
                ));
            } else {
                spans.push(Span::styled(
                    format!(" {} ", tab.label()),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            spans
        })
        .collect();

    let status_color = match req.status {
        200..=299 => Color::Green,
        300..=399 => Color::Cyan,
        400..=499 => Color::Yellow,
        _ => Color::Red,
    };

    let title_line = Line::from(vec![
        Span::styled(
            format!("  {} {} ", req.method, req.path),
            Style::default().fg(Color::Rgb(0, 200, 255)).bold(),
        ),
        Span::styled(format!("{}", req.status), Style::default().fg(status_color).bold()),
        Span::styled(
            format!("  {}  ", format_request_duration(req.duration)),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let tab_line = Line::from(tab_titles);
    f.render_widget(Paragraph::new(vec![title_line, tab_line]), outer_chunks[0]);

    // ── Tab content ──
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    match state.detail_tab {
        DetailTab::Request => {
            lines.push(Line::from(Span::styled(
                format!("  \u{2500}\u{2500} Headers ({}) \u{2500}\u{2500}", req.request_headers.len()),
                header_style,
            )));
            lines.push(Line::from(""));
            for h in &req.request_headers {
                lines.push(Line::from(vec![
                    Span::styled(format!("  {}  ", h.name), label_style),
                    Span::styled(truncate_header_value(&h.value), value_style),
                ]));
            }
            if let Some(body) = &req.request_body {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("  \u{2500}\u{2500} Body ({}) \u{2500}\u{2500}", format_bytes(req.request_size)),
                    header_style,
                )));
                lines.push(Line::from(""));
                for line in body.lines() {
                    lines.push(Line::from(Span::styled(format!("  {}", line), value_style)));
                }
            }
        }
        DetailTab::Response => {
            lines.push(Line::from(Span::styled(
                format!("  \u{2500}\u{2500} Headers ({}) \u{2500}\u{2500}", req.response_headers.len()),
                header_style,
            )));
            lines.push(Line::from(""));
            for h in &req.response_headers {
                lines.push(Line::from(vec![
                    Span::styled(format!("  {}  ", h.name), label_style),
                    Span::styled(truncate_header_value(&h.value), value_style),
                ]));
            }
            if let Some(body) = &req.response_body {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("  \u{2500}\u{2500} Body ({}) \u{2500}\u{2500}", format_bytes(req.response_size)),
                    header_style,
                )));
                lines.push(Line::from(""));
                for line in body.lines() {
                    lines.push(Line::from(Span::styled(format!("  {}", line), value_style)));
                }
            }
            if req.response_headers.is_empty() && req.response_body.is_none() {
                lines.push(Line::from(Span::styled("  No response data", Style::default().fg(Color::DarkGray))));
            }
        }
        DetailTab::Info => {
            lines.push(Line::from(Span::styled("  \u{2500}\u{2500} General \u{2500}\u{2500}", header_style)));
            lines.push(Line::from(""));
            let general_fields: Vec<(&str, Span)> = vec![
                ("  Method     ", Span::styled(
                    format!("{} {}", req.method, req.path),
                    Style::default().fg(Color::Rgb(0, 200, 255)).bold(),
                )),
                ("  Status     ", Span::styled(req.status.to_string(), Style::default().fg(status_color).bold())),
                ("  Duration   ", Span::raw(format_request_duration(req.duration))),
                ("  Remote IP  ", Span::raw(req.remote_ip.as_deref().unwrap_or("-").to_string())),
                ("  Country    ", Span::raw(req.country.as_deref().unwrap_or("-").to_string())),
                ("  User-Agent ", Span::raw(req.user_agent.as_deref().unwrap_or("-").to_string())),
                ("  Req Size   ", Span::raw(format_bytes(req.request_size))),
                ("  Resp Size  ", Span::raw(format_bytes(req.response_size))),
            ];
            for (label, value) in general_fields {
                lines.push(Line::from(vec![Span::styled(label, label_style), value]));
            }
            if req.is_mock {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "  \u{25CF} Served from mock rule",
                    Style::default().fg(Color::Yellow).bold(),
                )));
            }
        }
        DetailTab::WebSocket => {
            if req.ws_frames.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  No WebSocket frames captured",
                    Style::default().fg(Color::DarkGray),
                )));
            } else {
                lines.push(Line::from(Span::styled(
                    format!("  \u{2500}\u{2500} WebSocket Frames ({}) \u{2500}\u{2500}", req.ws_frames.len()),
                    header_style,
                )));
                lines.push(Line::from(""));
                let ws_start = req.ws_frames.first().map(|fr| fr.timestamp);
                for frame in &req.ws_frames {
                    let (arrow, color) = match frame.direction {
                        crate::event::WsDirection::ClientToServer => ("\u{2192}", Color::Cyan),
                        crate::event::WsDirection::ServerToClient => ("\u{2190}", Color::Magenta),
                    };
                    let opcode_str = match frame.opcode {
                        crate::event::WsOpcode::Text => "Text",
                        crate::event::WsOpcode::Binary => "Binary",
                        crate::event::WsOpcode::Ping => "Ping",
                        crate::event::WsOpcode::Pong => "Pong",
                        crate::event::WsOpcode::Close => "Close",
                    };
                    let payload_str = match &frame.payload_preview {
                        Some(preview) => {
                            let first_line = preview.lines().next().unwrap_or("");
                            if first_line.len() > 80 { format!("{}...", &first_line[..77]) } else { first_line.to_string() }
                        }
                        None => {
                            if frame.payload_size > 0 { format!("({})", format_bytes(frame.payload_size)) } else { String::new() }
                        }
                    };
                    let elapsed = ws_start.map(|s| frame.timestamp.duration_since(s)).unwrap_or_default();
                    let time_str = format!("+{:.1}s", elapsed.as_secs_f64());
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {} ", arrow), Style::default().fg(color).bold()),
                        Span::styled(format!("{:<6} ", opcode_str), Style::default().fg(color)),
                        Span::styled(format!("{} ", payload_str), value_style),
                        Span::styled(format!(" {}", time_str), Style::default().fg(Color::DarkGray)),
                    ]));
                }
            }
        }
    }

    let detail = Paragraph::new(lines)
        .scroll((state.detail_scroll, 0))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(ratatui::widgets::BorderType::Double)
                .border_style(Style::default().fg(Color::Rgb(60, 60, 100))),
        );

    f.render_widget(detail, outer_chunks[1]);

    // ── Footer: flash message or keybinding hints ──
    let is_flash = state.flash.as_ref().is_some_and(|(_, at)| {
        at.elapsed() < std::time::Duration::from_secs(3)
    });
    let footer_line = if is_flash {
        if let Some((msg, _)) = &state.flash {
            Line::from(vec![
                Span::styled(" \u{2713} ", Style::default().fg(Color::Green).bold()),
                Span::styled(msg.as_str(), Style::default().fg(Color::Green).bold()),
            ])
        } else {
            detail_hints()
        }
    } else {
        detail_hints()
    };
    f.render_widget(Paragraph::new(footer_line), outer_chunks[2]);
}

fn detail_hints() -> Line<'static> {
    let dim = Style::default().fg(Color::DarkGray);
    Line::from(Span::styled(
        " Esc back  \u{2190}\u{2192} tabs  \u{2191}\u{2193} scroll  c cURL  r replay  m mock",
        dim,
    ))
}
