use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use super::state::AppState;
use crate::qr;

pub fn render_quit(f: &mut Frame, area: Rect) {
    let accent = Color::Rgb(0, 255, 220);
    let w: u16 = 40;
    let h: u16 = 5;
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let popup = Rect::new(x, y, w.min(area.width), h.min(area.height));

    f.render_widget(ratatui::widgets::Clear, popup);

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Stop tunnel and quit? ", Style::default().fg(Color::White).bold()),
            Span::styled("y", Style::default().fg(accent).bold()),
            Span::styled("/", Style::default().fg(Color::DarkGray)),
            Span::styled("n", Style::default().fg(Color::White).bold()),
        ]),
    ];

    let block = Block::default()
        .title(Span::styled(" Quit ", Style::default().fg(Color::Yellow).bold()))
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Double)
        .border_style(Style::default().fg(Color::Rgb(60, 60, 100)));

    f.render_widget(Paragraph::new(lines).block(block), popup);
}

pub fn render_help(f: &mut Frame, area: Rect) {
    let accent = Style::default().fg(Color::Rgb(0, 255, 220)).bold();
    let key_style = Style::default().fg(Color::White).bold();
    let desc_style = Style::default().fg(Color::Gray);
    let section_style = accent;

    let bindings: Vec<Line> = vec![
        Line::from(Span::styled("  \u{2500}\u{2500} Navigation \u{2500}\u{2500}", section_style)),
        Line::from(""),
        Line::from(vec![Span::styled("  \u{2191}\u{2193} j/k      ", key_style), Span::styled("Select request / scroll", desc_style)]),
        Line::from(vec![Span::styled("  PgUp/PgDn   ", key_style), Span::styled("Scroll by page", desc_style)]),
        Line::from(vec![Span::styled("  Home/End    ", key_style), Span::styled("Jump to first / last", desc_style)]),
        Line::from(vec![Span::styled("  Enter       ", key_style), Span::styled("Open request detail", desc_style)]),
        Line::from(vec![Span::styled("  Esc         ", key_style), Span::styled("Back / clear filter", desc_style)]),
        Line::from(""),
        Line::from(Span::styled("  \u{2500}\u{2500} Actions \u{2500}\u{2500}", section_style)),
        Line::from(""),
        Line::from(vec![Span::styled("  c           ", key_style), Span::styled("Copy URL (list) / cURL (detail)", desc_style)]),
        Line::from(vec![Span::styled("  C           ", key_style), Span::styled("Copy cURL command (list view)", desc_style)]),
        Line::from(vec![Span::styled("  r           ", key_style), Span::styled("Replay request to localhost (detail view)", desc_style)]),
        Line::from(vec![Span::styled("  R           ", key_style), Span::styled("Replay request to tunnel (detail view)", desc_style)]),
        Line::from(vec![Span::styled("  e           ", key_style), Span::styled("Export requests to HAR file", desc_style)]),
        Line::from(vec![Span::styled("  m           ", key_style), Span::styled("Mock this endpoint (detail view)", desc_style)]),
        Line::from(""),
        Line::from(Span::styled("  \u{2500}\u{2500} Diff \u{2500}\u{2500}", section_style)),
        Line::from(""),
        Line::from(vec![Span::styled("  Space       ", key_style), Span::styled("Mark request for comparison", desc_style)]),
        Line::from(vec![Span::styled("  d           ", key_style), Span::styled("Diff marked vs selected", desc_style)]),
        Line::from(""),
        Line::from(Span::styled("  \u{2500}\u{2500} Watch & Filter \u{2500}\u{2500}", section_style)),
        Line::from(""),
        Line::from(vec![Span::styled("  w           ", key_style), Span::styled("Watch endpoint (auto-open detail)", desc_style)]),
        Line::from(vec![Span::styled("  /           ", key_style), Span::styled("Filter requests", desc_style)]),
        Line::from(vec![Span::styled("  S           ", key_style), Span::styled("Settings (tunnel token, custom domain)", desc_style)]),
        Line::from(vec![Span::styled("  s           ", key_style), Span::styled("Show QR code", desc_style)]),
        Line::from(vec![Span::styled("  h           ", key_style), Span::styled("This help screen", desc_style)]),
        Line::from(vec![Span::styled("  q           ", key_style), Span::styled("Quit", desc_style)]),
        Line::from(vec![Span::styled("  Ctrl+C      ", key_style), Span::styled("Stop tunnel and exit", desc_style)]),
    ];

    let h = bindings.len() as u16 + 4;
    let w: u16 = 62;
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let popup = Rect::new(x, y, w.min(area.width), h.min(area.height));

    f.render_widget(ratatui::widgets::Clear, popup);

    let block = Block::default()
        .title(Span::styled(" Keybindings ", accent))
        .title_bottom(Span::styled(" any key to close ", Style::default().fg(Color::DarkGray)))
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Double)
        .border_style(Style::default().fg(Color::Rgb(60, 60, 100)));

    f.render_widget(Paragraph::new(bindings).block(block), popup);
}

pub fn render_settings(f: &mut Frame, state: &AppState, area: Rect) {
    let accent = Color::Rgb(0, 255, 220);
    let label_style = Style::default().fg(Color::DarkGray);
    let active_label = Style::default().fg(Color::White).bold();
    let dim = Style::default().fg(Color::DarkGray);
    let cursor_style = Style::default().fg(accent);

    // Field 0-3 = API fields, field 4 = enable/disable toggle
    let field_labels = ["API Token", "Account ID", "Zone ID", "Subdomain"];
    let field_hints = [
        "cloudflare.com/profile/api-tokens (Tunnel:Edit + DNS:Edit)",
        "dashboard.cloudflare.com \u{2192} Account ID (right sidebar)",
        "dashboard.cloudflare.com \u{2192} select domain \u{2192} Zone ID",
        "e.g. \"tunnel\" \u{2192} *.tunnel.yourdomain.com",
    ];
    let saved_values = [
        &state.settings.api_token,
        &state.settings.account_id,
        &state.settings.zone_id,
        &state.settings.base_subdomain,
    ];

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  \u{2500}\u{2500} Custom Domain Setup \u{2500}\u{2500}",
            Style::default().fg(accent).bold(),
        )),
        Line::from(""),
    ];

    // Render the 4 API fields
    for (i, label) in field_labels.iter().enumerate() {
        let is_active = state.settings_field == i;
        let input = &state.settings_inputs[i];
        let saved = saved_values[i];

        let lbl = if is_active { active_label } else { label_style };
        let arrow = if is_active { "\u{25B6} " } else { "  " };
        let padded_label = format!("{}{:<12}", arrow, label);

        if !input.is_empty() {
            let display = if input.len() > 34 {
                format!("...{}", &input[input.len() - 31..])
            } else {
                input.clone()
            };
            let mut spans = vec![
                Span::styled(padded_label, lbl),
                Span::styled(display, Style::default().fg(Color::White)),
            ];
            if is_active {
                spans.push(Span::styled("\u{2588}", cursor_style));
            }
            lines.push(Line::from(spans));
        } else if !saved.is_empty() {
            let masked = if i == 0 {
                if saved.len() > 12 {
                    format!("{}...{}", &saved[..4], &saved[saved.len() - 4..])
                } else {
                    "\u{2022}".repeat(saved.len().min(8))
                }
            } else if saved.len() > 34 {
                format!("{}...", &saved[..31])
            } else {
                saved.clone()
            };
            let mut spans = vec![
                Span::styled(padded_label, lbl),
                Span::styled(masked, Style::default().fg(Color::White)),
                Span::styled("  \u{2713}", Style::default().fg(Color::Green)),
            ];
            if is_active {
                spans.push(Span::styled(" \u{2588}", cursor_style));
            }
            lines.push(Line::from(spans));
        } else {
            let mut spans = vec![Span::styled(padded_label, lbl)];
            if is_active {
                spans.push(Span::styled("\u{2588}", cursor_style));
            }
            lines.push(Line::from(spans));
        }

        // Show hint below the active field
        if is_active {
            lines.push(Line::from(Span::styled(
                format!("               {}", field_hints[i]),
                dim,
            )));
        }
    }

    lines.push(Line::from(""));

    // Enable/disable toggle (field 4)
    let toggle_active = state.settings_field == 4;
    let toggle_lbl = if toggle_active { active_label } else { label_style };
    let toggle_arrow = if toggle_active { "\u{25B6} " } else { "  " };
    let enabled = state.settings.custom_domain_enabled;
    let (toggle_icon, toggle_text, toggle_color) = if enabled {
        ("\u{25C9}", "Enabled", Color::Green)
    } else {
        ("\u{25CB}", "Disabled", Color::DarkGray)
    };
    lines.push(Line::from(vec![
        Span::styled(format!("{}{:<12}", toggle_arrow, "Mode"), toggle_lbl),
        Span::styled(
            format!("{} Custom domain: {}", toggle_icon, toggle_text),
            Style::default().fg(toggle_color).bold(),
        ),
    ]));
    if toggle_active {
        lines.push(Line::from(Span::styled(
            "               Space to toggle \u{2014} uses trycloudflare.com when off",
            dim,
        )));
    }

    // Status summary
    lines.push(Line::from(""));
    if enabled && state.settings.api_fields_complete() {
        lines.push(Line::from(vec![
            Span::styled("  Status   ", label_style),
            Span::styled(
                "Ready",
                Style::default().fg(Color::Green).bold(),
            ),
            Span::styled(
                " \u{2014} restart cfproxy to use custom domain",
                dim,
            ),
        ]));
    } else if enabled && !state.settings.api_fields_complete() {
        lines.push(Line::from(vec![
            Span::styled("  Status   ", label_style),
            Span::styled(
                "Incomplete",
                Style::default().fg(Color::Yellow).bold(),
            ),
            Span::styled(
                " \u{2014} fill in required fields above",
                dim,
            ),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("  Status   ", label_style),
            Span::styled("Using ", dim),
            Span::styled(
                "trycloudflare.com",
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(" (random URL each run)", dim),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  One-time setup: cfproxy creates a persistent tunnel and",
        dim,
    )));
    lines.push(Line::from(Span::styled(
        "  wildcard DNS. Use --host <name> for a specific subdomain.",
        dim,
    )));

    let h = lines.len() as u16 + 4;
    let w: u16 = 66;
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let popup = Rect::new(x, y, w.min(area.width), h.min(area.height));

    f.render_widget(ratatui::widgets::Clear, popup);

    let block = Block::default()
        .title(Span::styled(
            " Settings ",
            Style::default().fg(accent).bold(),
        ))
        .title_bottom(Span::styled(
            " Tab/\u{2191}\u{2193} navigate \u{00B7} Enter save \u{00B7} x clear \u{00B7} Esc close ",
            dim,
        ))
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Double)
        .border_style(Style::default().fg(Color::Rgb(60, 60, 100)));

    f.render_widget(Paragraph::new(lines).block(block), popup);
}

pub fn render_qr(f: &mut Frame, url: &str, area: Rect) {
    let qr_lines = qr::render_qr_lines(url);
    let qr_h = qr_lines.len() as u16 + 4;
    let qr_w = qr_lines.first().map(|l| l.chars().count()).unwrap_or(20) as u16 + 4;

    let x = area.x + area.width.saturating_sub(qr_w) / 2;
    let y = area.y + area.height.saturating_sub(qr_h) / 2;
    let popup = Rect::new(x, y, qr_w.min(area.width), qr_h.min(area.height));

    f.render_widget(ratatui::widgets::Clear, popup);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));
    for ql in &qr_lines {
        lines.push(Line::from(
            Span::styled(format!(" {} ", ql), Style::default().fg(Color::White)),
        ));
    }
    lines.push(Line::from(""));

    let accent = Style::default().fg(Color::Rgb(0, 255, 220)).bold();
    let block = Block::default()
        .title(Span::styled(" Scan QR Code ", accent))
        .title_bottom(Span::styled(" any key to close ", Style::default().fg(Color::DarkGray)))
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Double)
        .border_style(Style::default().fg(Color::Rgb(60, 60, 100)));

    f.render_widget(Paragraph::new(lines).block(block).alignment(Alignment::Center), popup);
}

pub fn render_overlays(f: &mut Frame, state: &AppState, area: Rect) {
    if state.confirm_quit {
        render_quit(f, area);
    } else if state.show_settings {
        render_settings(f, state, area);
    } else if state.show_help {
        render_help(f, area);
    } else if state.show_qr {
        if let Some(url) = &state.url {
            render_qr(f, url, area);
        }
    }
}
