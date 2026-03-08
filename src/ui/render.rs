use std::time::Duration;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use super::helpers::format_duration;
use super::state::{AppState, ConnectionStatus};

pub fn render_banner(f: &mut Frame, area: Rect) {
    let grid = banner_pixels();
    let rows = grid.len();
    let cols = if rows > 0 { grid[0].len() } else { 0 };

    let pad = 2;
    let inner_w = cols + pad * 2;
    let total_w = inner_w + 3;
    let shadow_style = Style::default().fg(Color::Rgb(30, 20, 60));

    let mut lines: Vec<Line> = Vec::new();

    // Top border
    {
        let mut spans = Vec::new();
        spans.push(Span::styled("\u{2554}", Style::default().fg(banner_gradient(0, cols))));
        for i in 0..inner_w {
            let col_idx = if i < pad { 0 } else if i >= pad + cols { cols.saturating_sub(1) } else { i - pad };
            spans.push(Span::styled("\u{2550}", Style::default().fg(banner_gradient(col_idx, cols))));
        }
        spans.push(Span::styled("\u{2557}", Style::default().fg(banner_gradient(cols.saturating_sub(1), cols))));
        spans.push(Span::raw(" "));
        lines.push(Line::from(spans));
    }

    // Content lines
    for pair in (0..rows).step_by(2) {
        let mut spans: Vec<Span> = Vec::new();
        spans.push(Span::styled("\u{2551}", Style::default().fg(banner_gradient(0, cols))));
        spans.push(Span::raw(" ".repeat(pad)));
        for (col, &top) in grid[pair].iter().enumerate() {
            let bot = if pair + 1 < rows { grid[pair + 1][col] } else { false };
            let color = banner_gradient(col, cols);
            let (ch, style) = match (top, bot) {
                (true, true) => ('\u{2588}', Style::default().fg(color)),
                (true, false) => ('\u{2580}', Style::default().fg(color)),
                (false, true) => ('\u{2584}', Style::default().fg(color)),
                (false, false) => (' ', Style::default()),
            };
            spans.push(Span::styled(String::from(ch), style));
        }
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled("\u{2551}", Style::default().fg(banner_gradient(cols.saturating_sub(1), cols))));
        spans.push(Span::styled("\u{2593}", shadow_style));
        lines.push(Line::from(spans));
    }

    // Bottom border
    {
        let mut spans = Vec::new();
        spans.push(Span::styled("\u{255A}", Style::default().fg(banner_gradient(0, cols))));
        for i in 0..inner_w {
            let col_idx = if i < pad { 0 } else if i >= pad + cols { cols.saturating_sub(1) } else { i - pad };
            spans.push(Span::styled("\u{2550}", Style::default().fg(banner_gradient(col_idx, cols))));
        }
        spans.push(Span::styled("\u{255D}", Style::default().fg(banner_gradient(cols.saturating_sub(1), cols))));
        spans.push(Span::styled("\u{2593}", shadow_style));
        lines.push(Line::from(spans));
    }

    // Shadow
    {
        let shadow_w = total_w - 1;
        let shadow_str: String = std::iter::once(' ').chain(std::iter::repeat_n('\u{2593}', shadow_w)).collect();
        lines.push(Line::from(Span::styled(shadow_str, shadow_style)));
    }

    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
}

fn banner_gradient(col: usize, total: usize) -> Color {
    let t = col as f32 / total.max(1) as f32;
    let r = (0.0 + t * 200.0) as u8;
    let g = (255.0 - t * 180.0) as u8;
    let b = (220.0 + t * 35.0).min(255.0) as u8;
    Color::Rgb(r, g, b)
}

fn banner_pixels() -> Vec<Vec<bool>> {
    #[rustfmt::skip]
    let letters: &[&[&str]] = &[
        &["011110","110011","110000","110000","110000","110011","011110"],
        &["111111","110000","110000","111100","110000","110000","110000"],
        &["111110","110011","110011","111110","110000","110000","110000"],
        &["111110","110011","110011","111100","110110","110011","110011"],
        &["011110","110011","110011","110011","110011","110011","011110"],
        &["110011","110011","011110","001100","011110","110011","110011"],
        &["110011","110011","011110","001100","001100","001100","001100"],
    ];

    let letter_h = 7;
    let gap = 2;
    let total_w: usize = letters.iter().map(|l| l[0].len()).sum::<usize>() + gap * (letters.len() - 1);
    let mut grid = vec![vec![false; total_w]; letter_h];

    let mut col_offset = 0;
    for (i, letter) in letters.iter().enumerate() {
        if i > 0 { col_offset += gap; }
        for (row, bits) in letter.iter().enumerate() {
            for (c, ch) in bits.chars().enumerate() {
                grid[row][col_offset + c] = ch == '1';
            }
        }
        col_offset += letter[0].len();
    }
    grid
}

fn sparkline_text(data: &[u64], width: usize) -> String {
    let blocks = [' ', '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}', '\u{2588}'];
    let max = data.iter().copied().max().unwrap_or(0).max(1);
    let start = data.len().saturating_sub(width);
    data[start..].iter().map(|&v| {
        let idx = ((v as f64 / max as f64) * 8.0).round().min(8.0) as usize;
        blocks[idx]
    }).collect()
}

pub fn render_info_panel(f: &mut Frame, state: &AppState, area: Rect) {
    let label = Style::default().fg(Color::DarkGray);

    let (status_text, status_color) = match &state.status {
        ConnectionStatus::Connecting => ("\u{25CF} connecting", Color::Yellow),
        ConnectionStatus::Connected => ("\u{25CF} online", Color::Green),
        ConnectionStatus::Disconnected(_) => ("\u{25CF} disconnected", Color::Red),
    };

    let edge = state.edge_location.as_deref().unwrap_or("...");
    let uptime = format_duration(state.uptime());

    let rps_data = state.stats.rps_data();
    let max_rps = rps_data.iter().copied().max().unwrap_or(0);

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("    Status ", label),
            Span::styled(status_text, Style::default().fg(status_color).bold()),
        ]),
        Line::from(vec![
            Span::styled("      Edge ", label),
            Span::raw(edge.to_string()),
            Span::styled("  Up ", label),
            Span::raw(uptime.to_string()),
        ]),
    ];

    // Traffic sparkline
    let mut traffic = vec![
        Span::styled("   Traffic ", label),
    ];
    if max_rps > 0 {
        traffic.push(Span::styled(
            sparkline_text(&rps_data, 20),
            Style::default().fg(Color::Rgb(0, 200, 255)),
        ));
        traffic.push(Span::styled(format!(" peak {}", max_rps), label));
    } else {
        traffic.push(Span::styled("\u{2014}", label));
    }
    lines.push(Line::from(traffic));

    f.render_widget(Paragraph::new(lines), area);
}

pub fn render_url_bar(f: &mut Frame, state: &AppState, area: Rect) {
    let url = state.url.as_deref().unwrap_or("waiting for tunnel...");
    let url_style = Style::default().fg(Color::Rgb(180, 100, 255)).bold().underlined();
    let label = Style::default().fg(Color::DarkGray);

    let line = Line::from(vec![
        Span::styled(" \u{2192} ", label),
        Span::styled(url, url_style),
        Span::styled("  \u{2192}  ", label),
        Span::styled(format!("http://localhost:{}", state.port), label),
    ]);

    f.render_widget(Paragraph::new(line), area);
}

pub fn render_compact_header(f: &mut Frame, state: &AppState, area: Rect) {
    let (status_text, status_color) = match &state.status {
        ConnectionStatus::Connecting => ("\u{25CF} connecting...", Color::Yellow),
        ConnectionStatus::Connected => ("\u{25CF} online", Color::Green),
        ConnectionStatus::Disconnected(_) => ("\u{25CF} disconnected", Color::Red),
    };
    let label = Style::default().fg(Color::DarkGray);
    let uptime = format_duration(state.uptime());
    let edge = state.edge_location.as_deref().unwrap_or("...");

    let rps_data = state.stats.rps_data();
    let max_rps = rps_data.iter().copied().max().unwrap_or(0);

    let mut status_line = vec![
        Span::styled("  ", Style::default()),
        Span::styled(status_text, Style::default().fg(status_color).bold()),
        Span::raw("  "),
        Span::styled("Edge ", label),
        Span::raw(edge),
        Span::raw("  "),
        Span::styled("Up ", label),
        Span::raw(uptime),
        Span::styled("  \u{2192} ", label),
        Span::styled(format!("localhost:{}", state.port), label),
    ];
    if max_rps > 0 {
        status_line.push(Span::raw("  "));
        status_line.push(Span::styled(sparkline_text(&rps_data, 15), Style::default().fg(Color::Rgb(0, 200, 255))));
        status_line.push(Span::styled(format!(" peak {}", max_rps), label));
    }

    let lines = vec![
        Line::from(status_line),
        Line::from(""),
    ];

    f.render_widget(Paragraph::new(lines), area);
}

pub fn render_stats_bar(f: &mut Frame, state: &AppState, area: Rect) {
    let s = &state.stats;
    let mut spans = vec![
        Span::styled("Total ", Style::default().fg(Color::DarkGray)),
        Span::styled(s.total.to_string(), Style::default().bold()),
        Span::raw("  "),
        Span::styled("2xx ", Style::default().fg(Color::DarkGray)),
        Span::styled(s.status_2xx.to_string(), Style::default().fg(Color::Green)),
        Span::raw("  "),
        Span::styled("3xx ", Style::default().fg(Color::DarkGray)),
        Span::styled(s.status_3xx.to_string(), Style::default().fg(Color::Cyan)),
        Span::raw("  "),
        Span::styled("4xx ", Style::default().fg(Color::DarkGray)),
        Span::styled(s.status_4xx.to_string(), Style::default().fg(Color::Yellow)),
        Span::raw("  "),
        Span::styled("5xx ", Style::default().fg(Color::DarkGray)),
        Span::styled(s.status_5xx.to_string(), Style::default().fg(Color::Red)),
        Span::raw("  "),
        Span::styled("Avg ", Style::default().fg(Color::DarkGray)),
        Span::raw(format!("{}ms", s.avg_ms())),
    ];

    if !state.filter.is_empty() && !state.filter_active {
        spans.push(Span::styled("  \u{2502} ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            format!("/{}", state.filter),
            Style::default().fg(Color::Rgb(0, 255, 220)),
        ));
        spans.push(Span::styled(
            "  Esc clear",
            Style::default().fg(Color::DarkGray),
        ));
    }

    let line = Line::from(spans);

    f.render_widget(
        Paragraph::new(line).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Color::DarkGray)),
        ),
        area,
    );
}

pub fn render_footer(f: &mut Frame, state: &AppState, area: Rect) {
    let cf_ver = state.version.as_deref().unwrap_or("...");
    let right_side = if let Some(ref pattern) = state.watch {
        format!("q quit \u{00B7} h help \u{00B7} watching: {} \u{00B7} w clear \u{00B7} v{} \u{00B7} cf {}", pattern, env!("CARGO_PKG_VERSION"), cf_ver)
    } else {
        format!("q quit \u{00B7} h help \u{00B7} v{} \u{00B7} cf {} ", env!("CARGO_PKG_VERSION"), cf_ver)
    };

    let right_style = Style::default().fg(Color::DarkGray);
    let accent = Style::default().fg(Color::Rgb(0, 255, 220)).bold();
    let dim = Style::default().fg(Color::DarkGray);
    let available = area.width.saturating_sub(2) as usize;
    let right_len = right_side.len();

    let center_hint = "filter by path, method, status, type, ip, ua";
    let center_len = center_hint.len();

    let line = if state.filter_active {
        // Left: filter input, Center: hint (fixed), Right: quit/help
        let left = format!(" / {}\u{2588}", state.filter);
        let left_len = left.len();

        // Position center hint in the middle of available space
        let center_pos = available / 2 - center_len / 2;
        let left_pad = center_pos.saturating_sub(left_len);
        let right_pad = available.saturating_sub(center_pos + center_len + right_len);

        Line::from(vec![
            Span::styled(left, accent),
            Span::raw(" ".repeat(left_pad)),
            Span::styled(center_hint, dim),
            Span::raw(" ".repeat(right_pad)),
            Span::styled(right_side, right_style),
        ])
    } else {
        let is_flash = state.flash.as_ref().is_some_and(|(_, at)| at.elapsed() < Duration::from_secs(3));
        let left_side = if is_flash {
            if let Some((msg, _)) = &state.flash {
                format!(" \u{2713} {}", msg)
            } else {
                request_hints(state)
            }
        } else {
            request_hints(state)
        };
        let left_len = left_side.len();
        let padding = available.saturating_sub(left_len + right_len);

        Line::from(vec![
            Span::styled(left_side, accent),
            Span::raw(" ".repeat(padding)),
            Span::styled(right_side, right_style),
        ])
    };

    f.render_widget(
        Paragraph::new(line).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Color::DarkGray)),
        ),
        area,
    );
}

fn request_hints(state: &AppState) -> String {
    let mut parts: Vec<&str> = Vec::new();

    // Always show core actions
    parts.push("/ filter");
    parts.push("w watch");
    parts.push("e export");

    // Add selection-aware actions
    if state.selected.is_some() {
        parts.push("\u{21B5} detail");
        parts.push("Space mark");
    } else {
        parts.push("\u{2191}\u{2193} select");
    }

    // Diff only when both marked and selected
    if state.selected.is_some() && state.marked.is_some() {
        parts.push("d diff");
    }

    // Active filter indicator
    if !state.filter.is_empty() {
        parts.push("Esc clear");
    }

    format!(" {}", parts.join("  "))
}

pub fn render_diff_view(f: &mut Frame, state: &AppState, area: Rect) {
    let indices = state.filtered_indices();

    let (req_a, req_b) = match (state.marked, state.selected) {
        (Some(m), Some(s)) => {
            let a = indices.get(m).and_then(|&ri| state.requests.iter().rev().nth(ri));
            let b = indices.get(s).and_then(|&ri| state.requests.iter().rev().nth(ri));
            match (a, b) {
                (Some(a), Some(b)) => (a, b),
                _ => {
                    f.render_widget(Paragraph::new("Could not resolve requests for diff").alignment(Alignment::Center), area);
                    return;
                }
            }
        }
        _ => {
            f.render_widget(Paragraph::new("No requests selected for diff").alignment(Alignment::Center), area);
            return;
        }
    };

    let diff_result = crate::diff::diff_requests(req_a, req_b);
    let accent = Color::Rgb(0, 255, 220);
    let header_style = Style::default().fg(accent).bold();

    let lines: Vec<Line> = diff_result
        .iter()
        .map(|d| match d {
            crate::diff::DiffLine::Same(s) => {
                Line::from(Span::styled(format!("  {}", s), Style::default().fg(Color::White)))
            }
            crate::diff::DiffLine::Added(s) => {
                Line::from(Span::styled(format!("+ {}", s), Style::default().fg(Color::Green)))
            }
            crate::diff::DiffLine::Removed(s) => {
                Line::from(Span::styled(format!("- {}", s), Style::default().fg(Color::Red)))
            }
        })
        .collect();

    let title = format!(" Diff: {} {} vs {} {} ", req_a.method, req_a.path, req_b.method, req_b.path);

    f.render_widget(
        Paragraph::new(lines)
            .scroll((state.detail_scroll, 0))
            .block(
                Block::default()
                    .title(Span::styled(title, header_style))
                    .title_bottom(Span::styled(" Esc back  \u{2191}\u{2193} scroll ", Style::default().fg(Color::DarkGray)))
                    .borders(Borders::ALL)
                    .border_type(ratatui::widgets::BorderType::Double)
                    .border_style(Style::default().fg(Color::Rgb(60, 60, 100))),
            ),
        area,
    );
}
