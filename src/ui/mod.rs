mod detail;
mod helpers;
mod overlays;
mod render;
mod requests;
pub mod state;

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::prelude::*;
use tokio::sync::mpsc;

use crate::event::{HttpRequest, TunnelEvent};
use crate::metrics;
use crate::mock::{MockRule, MockRules};

use state::{AppState, DetailTab, ViewMode};
use helpers::{copy_to_clipboard, replay_request, replay_to_url};

const TICK_RATE: Duration = Duration::from_secs(1);
const MIN_COLS: u16 = 120;
const MIN_ROWS: u16 = 35;

/// Query current terminal size, resize if too small, return original size for restore.
fn ensure_terminal_size() -> Option<(u16, u16)> {
    let (cols, rows) = crossterm::terminal::size().ok()?;
    if cols >= MIN_COLS && rows >= MIN_ROWS {
        return None;
    }
    let new_cols = cols.max(MIN_COLS);
    let new_rows = rows.max(MIN_ROWS);
    // ANSI escape: CSI 8 ; rows ; cols t — resize terminal window
    print!("\x1b[8;{};{}t", new_rows, new_cols);
    use std::io::Write;
    let _ = io::stdout().flush();
    // Short delay for terminal to process resize
    std::thread::sleep(Duration::from_millis(50));
    Some((cols, rows))
}

fn restore_terminal_size(original: (u16, u16)) {
    print!("\x1b[8;{};{}t", original.1, original.0);
    use std::io::Write;
    let _ = io::stdout().flush();
}

pub async fn run(port: u16, mut event_rx: mpsc::Receiver<TunnelEvent>, mock_rules: MockRules) -> crate::error::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    // Resize after entering alternate screen so scrollback stays clean
    let original_size = ensure_terminal_size();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = AppState::new(port, mock_rules);
    let mut last_tick = Instant::now();
    let mut last_metrics_fetch = Instant::now();

    loop {
        terminal.draw(|f| draw(f, &state))?;

        let timeout = TICK_RATE
            .checked_sub(last_tick.elapsed())
            .unwrap_or(Duration::ZERO);

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press
                    && !handle_key(&mut state, key.code, key.modifiers)
                {
                    break;
                }
            }
        }

        while let Ok(ev) = event_rx.try_recv() {
            state.apply_event(ev);
        }

        if last_tick.elapsed() >= TICK_RATE {
            last_tick = Instant::now();
            state.stats.tick();

            if let Some(metrics_port) = state.metrics_port {
                if last_metrics_fetch.elapsed() >= TICK_RATE {
                    last_metrics_fetch = Instant::now();
                    if let Ok(m) = metrics::fetch(metrics_port).await {
                        state.apply_event(TunnelEvent::Metrics(m));
                    }
                }
            }
        }
    }

    if let Some(orig) = original_size {
        restore_terminal_size(orig);
    }
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Returns false if the app should quit.
fn handle_key(state: &mut AppState, code: KeyCode, modifiers: crossterm::event::KeyModifiers) -> bool {
    // Quit confirmation dialog
    if state.confirm_quit {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => return false,
            _ => { state.confirm_quit = false; }
        }
        return true;
    }

    // Ctrl+C triggers quit confirmation
    if code == KeyCode::Char('c') && modifiers.contains(crossterm::event::KeyModifiers::CONTROL) {
        state.confirm_quit = true;
        return true;
    }

    // Settings overlay — handles its own input
    if state.show_settings {
        handle_settings_key(state, code);
        return true;
    }

    // Overlays — any key dismisses
    if state.show_help {
        state.show_help = false;
        return true;
    }
    if state.show_qr {
        state.show_qr = false;
        return true;
    }

    // Filter input mode
    if state.filter_active {
        match code {
            KeyCode::Esc => {
                state.filter_active = false;
                state.filter.clear();
                state.selected = None;
            }
            KeyCode::Enter => {
                state.filter_active = false;
                state.selected = None;
            }
            KeyCode::Backspace => {
                state.filter.pop();
                state.selected = None;
            }
            KeyCode::Char(c) => {
                state.filter.push(c);
                state.selected = None;
            }
            _ => {}
        }
        return true;
    }

    match state.view {
        ViewMode::List => handle_list_key(state, code),
        ViewMode::Detail => handle_detail_key(state, code),
        ViewMode::Diff => handle_diff_key(state, code),
    }

    true
}

fn handle_list_key(state: &mut AppState, code: KeyCode) {
    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            state.confirm_quit = true;
        }
        KeyCode::Char('c') => {
            if let Some(url) = &state.url {
                if copy_to_clipboard(url) {
                    state.set_flash("URL copied");
                }
            }
        }
        KeyCode::Char('C') => {
            if let Some(req) = resolve_selected(state) {
                let fallback = format!("http://localhost:{}", state.port);
                let base = state.url.as_deref().unwrap_or(&fallback);
                let curl = req.to_curl(base);
                if copy_to_clipboard(&curl) {
                    state.set_flash("cURL copied");
                } else {
                    state.set_flash("Clipboard unavailable");
                }
            }
        }
        KeyCode::Char('s') => {
            if state.url.is_some() {
                state.show_qr = true;
            }
        }
        KeyCode::Char('S') => {
            state.show_settings = true;
            state.settings_field = 0;
            state.settings_inputs = vec![String::new(); 4];
        }
        KeyCode::Char('h') => {
            state.show_help = true;
        }
        KeyCode::Char('/') => {
            state.filter_active = true;
            state.filter.clear();
            state.selected = None;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let indices = state.filtered_indices();
            let len = indices.len();
            if len > 0 {
                state.selected = Some(match state.selected {
                    None => 0,
                    Some(i) => (i + 1).min(len - 1),
                });
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let indices = state.filtered_indices();
            if !indices.is_empty() {
                state.selected = Some(match state.selected {
                    None => 0,
                    Some(i) => i.saturating_sub(1),
                });
            }
        }
        KeyCode::PageDown => {
            let indices = state.filtered_indices();
            let len = indices.len();
            if len > 0 {
                let page = 20;
                state.selected = Some(match state.selected {
                    None => page.min(len - 1),
                    Some(i) => (i + page).min(len - 1),
                });
            }
        }
        KeyCode::PageUp => {
            let indices = state.filtered_indices();
            if !indices.is_empty() {
                let page = 20;
                state.selected = Some(match state.selected {
                    None => 0,
                    Some(i) => i.saturating_sub(page),
                });
            }
        }
        KeyCode::Home => {
            if !state.filtered_indices().is_empty() {
                state.selected = Some(0);
            }
        }
        KeyCode::End => {
            let len = state.filtered_indices().len();
            if len > 0 {
                state.selected = Some(len - 1);
            }
        }
        KeyCode::Enter => {
            if state.selected.is_some() && !state.requests.is_empty() {
                state.detail_scroll = 0;
                state.detail_tab = DetailTab::Request;
                state.view = ViewMode::Detail;
            }
        }
        KeyCode::Char('e') => {
            let indices = state.filtered_indices();
            let refs: Vec<&crate::event::HttpRequest> = if !state.filter.is_empty() {
                indices.iter().map(|&i| &state.requests[i]).collect()
            } else {
                state.requests.iter().collect()
            };
            if !refs.is_empty() {
                let har = crate::har::to_har(&refs, env!("CARGO_PKG_VERSION"));
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let filename = format!("cfproxy-export-{}.har", ts);
                if std::fs::write(&filename, serde_json::to_string_pretty(&har).unwrap_or_default()).is_ok() {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(&filename, std::fs::Permissions::from_mode(0o600));
                    }
                    state.set_flash(&format!("Exported to {}", filename));
                } else {
                    state.set_flash("Failed to write HAR file");
                }
            } else {
                state.set_flash("No requests to export");
            }
        }
        KeyCode::Char(' ') => {
            if let Some(sel) = state.selected {
                if state.marked == Some(sel) {
                    state.marked = None;
                } else {
                    state.marked = Some(sel);
                }
            }
        }
        KeyCode::Char('d') => {
            if let (Some(marked), Some(selected)) = (state.marked, state.selected) {
                if marked != selected {
                    state.detail_scroll = 0;
                    state.view = ViewMode::Diff;
                }
            }
        }
        KeyCode::Char('w') => {
            if state.watch.is_some() {
                state.watch = None;
                state.set_flash("Watch cleared");
            } else if let Some(req) = state.selected.and_then(|i| {
                let indices = state.filtered_indices();
                indices.get(i).and_then(|&ri| state.requests.iter().rev().nth(ri))
            }) {
                let path = req.path.clone();
                state.set_flash(&format!("Watching: {}", path));
                state.watch = Some(path);
            }
        }
        KeyCode::Esc => {
            if !state.filter.is_empty() {
                state.filter.clear();
                state.selected = None;
            } else {
                state.selected = None;
            }
        }
        _ => {}
    }
}

/// Resolve the currently selected request, or None if selection is invalid.
fn resolve_selected(state: &AppState) -> Option<&HttpRequest> {
    state.selected.and_then(|i| {
        let indices = state.filtered_indices();
        indices.get(i).and_then(|&ri| state.requests.iter().rev().nth(ri))
    })
}

fn handle_detail_key(state: &mut AppState, code: KeyCode) {
    match code {
        KeyCode::Esc | KeyCode::Backspace => {
            state.view = ViewMode::List;
        }
        KeyCode::Char('q') => {
            state.view = ViewMode::List;
        }
        KeyCode::Char('c') => {
            if let Some(req) = resolve_selected(state) {
                let fallback = format!("http://localhost:{}", state.port);
                let base = state.url.as_deref().unwrap_or(&fallback);
                let curl = req.to_curl(base);
                if copy_to_clipboard(&curl) {
                    state.set_flash("cURL copied");
                } else {
                    state.set_flash("Clipboard unavailable");
                }
            } else {
                state.set_flash("No request selected");
            }
        }
        KeyCode::Char('r') => {
            if let Some(req) = resolve_selected(state) {
                replay_request(req, state.port);
                state.set_flash("Request replayed \u{2192} localhost");
            } else {
                state.set_flash("No request selected");
            }
        }
        KeyCode::Char('R') => {
            if let Some(req) = resolve_selected(state) {
                if let Some(url) = &state.url {
                    replay_to_url(req, url);
                    state.set_flash("Request replayed \u{2192} tunnel");
                } else {
                    state.set_flash("No tunnel URL yet");
                }
            } else {
                state.set_flash("No request selected");
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.detail_scroll = state.detail_scroll.saturating_add(1);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.detail_scroll = state.detail_scroll.saturating_sub(1);
        }
        KeyCode::PageDown => {
            state.detail_scroll = state.detail_scroll.saturating_add(20);
        }
        KeyCode::PageUp => {
            state.detail_scroll = state.detail_scroll.saturating_sub(20);
        }
        KeyCode::Home => {
            state.detail_scroll = 0;
        }
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => {
            let has_ws = state.selected.and_then(|i| {
                let indices = state.filtered_indices();
                indices.get(i).and_then(|&ri| state.requests.iter().rev().nth(ri))
            }).is_some_and(|r| r.is_websocket);
            let tabs = DetailTab::all(has_ws);
            if let Some(pos) = tabs.iter().position(|t| *t == state.detail_tab) {
                if pos + 1 < tabs.len() {
                    state.detail_tab = tabs[pos + 1];
                    state.detail_scroll = 0;
                }
            }
        }
        KeyCode::Left | KeyCode::Char('H') | KeyCode::BackTab => {
            let has_ws = state.selected.and_then(|i| {
                let indices = state.filtered_indices();
                indices.get(i).and_then(|&ri| state.requests.iter().rev().nth(ri))
            }).is_some_and(|r| r.is_websocket);
            let tabs = DetailTab::all(has_ws);
            if let Some(pos) = tabs.iter().position(|t| *t == state.detail_tab) {
                if pos > 0 {
                    state.detail_tab = tabs[pos - 1];
                    state.detail_scroll = 0;
                }
            }
        }
        KeyCode::Char('m') => {
            if let Some(req) = state.selected.and_then(|i| {
                let indices = state.filtered_indices();
                indices.get(i).and_then(|&ri| state.requests.iter().rev().nth(ri))
            }) {
                let method = req.method.clone();
                let path = req.path.clone();
                let status = req.status;
                let rule = MockRule {
                    path_pattern: path.clone(),
                    method: Some(method.clone()),
                    status,
                    content_type: "text/plain".to_string(),
                    body: String::new(),
                    hit_count: 0,
                };
                let mock_rules = state.mock_rules.clone();
                tokio::spawn(async move {
                    let mut rules = mock_rules.write().await;
                    rules.push(rule);
                });
                state.set_flash(&format!("Mocked: {} {} \u{2192} {}", method, path, status));
            }
        }
        _ => {}
    }
}

fn handle_diff_key(state: &mut AppState, code: KeyCode) {
    match code {
        KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('q') => {
            state.view = ViewMode::List;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.detail_scroll = state.detail_scroll.saturating_add(1);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.detail_scroll = state.detail_scroll.saturating_sub(1);
        }
        KeyCode::PageDown => {
            state.detail_scroll = state.detail_scroll.saturating_add(20);
        }
        KeyCode::PageUp => {
            state.detail_scroll = state.detail_scroll.saturating_sub(20);
        }
        KeyCode::Home => {
            state.detail_scroll = 0;
        }
        _ => {}
    }
}

fn handle_settings_key(state: &mut AppState, code: KeyCode) {
    // Fields 0-3 = text inputs, field 4 = enable/disable toggle
    const NUM_FIELDS: usize = 5;
    const TOGGLE_FIELD: usize = 4;

    match code {
        KeyCode::Esc => {
            state.show_settings = false;
            state.settings_inputs = vec![String::new(); 4];
        }
        KeyCode::Tab | KeyCode::Down => {
            if state.settings_field < TOGGLE_FIELD {
                apply_settings_field(state);
            }
            state.settings_field = (state.settings_field + 1) % NUM_FIELDS;
        }
        KeyCode::BackTab | KeyCode::Up => {
            if state.settings_field < TOGGLE_FIELD {
                apply_settings_field(state);
            }
            state.settings_field = if state.settings_field == 0 {
                NUM_FIELDS - 1
            } else {
                state.settings_field - 1
            };
        }
        KeyCode::Char(' ') if state.settings_field == TOGGLE_FIELD => {
            state.settings.custom_domain_enabled = !state.settings.custom_domain_enabled;
        }
        KeyCode::Enter => {
            if state.settings_field < TOGGLE_FIELD {
                apply_settings_field(state);
            }
            if state.settings.save().is_ok() {
                state.set_flash("Settings saved");
            } else {
                state.set_flash("Failed to save settings");
            }
            state.show_settings = false;
            state.settings_inputs = vec![String::new(); 4];
        }
        KeyCode::Backspace if state.settings_field < TOGGLE_FIELD => {
            let input = &mut state.settings_inputs[state.settings_field];
            if input.is_empty() {
                clear_settings_field(state);
            } else {
                input.pop();
            }
        }
        KeyCode::Char('x')
            if state.settings_field < TOGGLE_FIELD
                && state.settings_inputs[state.settings_field].is_empty() =>
        {
            clear_settings_field(state);
        }
        KeyCode::Char(c) if state.settings_field < TOGGLE_FIELD => {
            state.settings_inputs[state.settings_field].push(c);
        }
        _ => {}
    }
}

/// Apply the current input to the corresponding settings field.
fn apply_settings_field(state: &mut AppState) {
    let input = &state.settings_inputs[state.settings_field];
    if input.is_empty() {
        return;
    }
    let value = input.clone();
    match state.settings_field {
        0 => state.settings.api_token = value,
        1 => state.settings.account_id = value,
        2 => state.settings.zone_id = value,
        3 => state.settings.base_subdomain = value,
        _ => {}
    }
    state.settings_inputs[state.settings_field].clear();
}

/// Clear the saved value for the current settings field.
fn clear_settings_field(state: &mut AppState) {
    let field_name = match state.settings_field {
        0 => {
            state.settings.api_token.clear();
            "API Token"
        }
        1 => {
            state.settings.account_id.clear();
            "Account ID"
        }
        2 => {
            state.settings.zone_id.clear();
            "Zone ID"
        }
        3 => {
            state.settings.base_subdomain.clear();
            "Subdomain"
        }
        _ => return,
    };
    let _ = state.settings.save();
    state.set_flash(&format!("{} cleared", field_name));
}

fn draw(f: &mut Frame, state: &AppState) {
    let area = f.area();

    match state.view {
        ViewMode::List => {
            let wide = area.width >= 95;

            if wide {
                // Side-by-side: banner left, info panel right, URL on its own row
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(7),
                        Constraint::Length(1),
                        Constraint::Length(2),
                        Constraint::Min(5),
                        Constraint::Length(2),
                    ])
                    .split(area);

                let header = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Length(63),
                        Constraint::Min(25),
                    ])
                    .split(chunks[0]);

                render::render_banner(f, header[0]);
                render::render_info_panel(f, state, header[1]);
                render::render_url_bar(f, state, chunks[1]);
                render::render_stats_bar(f, state, chunks[2]);
                requests::render(f, state, chunks[3]);
                render::render_footer(f, state, chunks[4]);
            } else {
                // Narrow: compact stacked header (no banner), URL on its own row
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(2),
                        Constraint::Length(1),
                        Constraint::Length(2),
                        Constraint::Min(5),
                        Constraint::Length(2),
                    ])
                    .split(area);

                render::render_compact_header(f, state, chunks[0]);
                render::render_url_bar(f, state, chunks[1]);
                render::render_stats_bar(f, state, chunks[2]);
                requests::render(f, state, chunks[3]);
                render::render_footer(f, state, chunks[4]);
            }
        }
        ViewMode::Detail => {
            detail::render(f, state, area);
        }
        ViewMode::Diff => {
            render::render_diff_view(f, state, area);
        }
    }

    overlays::render_overlays(f, state, area);
}

#[cfg(test)]
mod tests {
    use super::state::*;
    use crate::event::{HttpRequest, TunnelEvent};
    use crate::metrics::TunnelMetrics;
    use std::time::Duration;
    use super::helpers::*;
    use crate::event::HeaderPair;

    #[test]
    fn build_replay_request_localhost_url() {
        let req = make_request("GET", "/api/test", 200);
        let (method, url, headers, body) = build_replay_request(&req, "http://127.0.0.1:3000");
        assert_eq!(method, "GET");
        assert_eq!(url, "http://127.0.0.1:3000/api/test");
        assert!(headers.is_empty());
        assert!(body.is_none());
    }

    #[test]
    fn build_replay_request_tunnel_url() {
        let req = make_request("POST", "/webhook", 200);
        let (method, url, _, _) = build_replay_request(&req, "https://abc-123.trycloudflare.com");
        assert_eq!(method, "POST");
        assert_eq!(url, "https://abc-123.trycloudflare.com/webhook");
    }

    #[test]
    fn build_replay_request_strips_trailing_slash() {
        let req = make_request("GET", "/path", 200);
        let (_, url, _, _) = build_replay_request(&req, "https://example.com/");
        assert_eq!(url, "https://example.com/path");
    }

    #[test]
    fn build_replay_request_filters_cf_headers() {
        let req = HttpRequest {
            method: "GET".into(), path: "/test".into(), status: 200,
            duration: Duration::from_millis(10),
            remote_ip: None, country: None, user_agent: None,
            request_headers: vec![
                HeaderPair { name: "content-type".into(), value: "application/json".into() },
                HeaderPair { name: "cf-connecting-ip".into(), value: "1.2.3.4".into() },
                HeaderPair { name: "x-custom".into(), value: "keep".into() },
                HeaderPair { name: "host".into(), value: "example.com".into() },
                HeaderPair { name: "cdn-loop".into(), value: "cloudflare".into() },
                HeaderPair { name: "x-forwarded-for".into(), value: "1.2.3.4".into() },
                HeaderPair { name: "x-forwarded-proto".into(), value: "https".into() },
            ],
            response_headers: Vec::new(),
            request_size: 0, response_size: 0,
            request_body: None, response_body: None,
            is_websocket: false, ws_frames: Vec::new(), is_mock: false, timestamp: chrono::Local::now(),
        };
        let (_, _, headers, _) = build_replay_request(&req, "http://localhost:3000");
        let names: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
        assert!(names.contains(&"content-type"));
        assert!(names.contains(&"x-custom"));
        assert!(!names.contains(&"cf-connecting-ip"));
        assert!(!names.contains(&"host"));
        assert!(!names.contains(&"cdn-loop"));
        assert!(!names.contains(&"x-forwarded-for"));
        assert!(!names.contains(&"x-forwarded-proto"));
        assert_eq!(headers.len(), 2);
    }

    #[test]
    fn build_replay_request_includes_body() {
        let req = HttpRequest {
            method: "POST".into(), path: "/data".into(), status: 200,
            duration: Duration::from_millis(10),
            remote_ip: None, country: None, user_agent: None,
            request_headers: Vec::new(), response_headers: Vec::new(),
            request_size: 0, response_size: 0,
            request_body: Some("{\"key\":\"value\"}".into()), response_body: None,
            is_websocket: false, ws_frames: Vec::new(), is_mock: false, timestamp: chrono::Local::now(),
        };
        let (_, _, _, body) = build_replay_request(&req, "http://localhost:3000");
        assert_eq!(body.as_deref(), Some("{\"key\":\"value\"}"));
    }

    #[test]
    fn app_state_new_defaults() {
        let state = AppState::new(3000, crate::mock::new_rules());
        assert_eq!(state.port, 3000);
        assert_eq!(state.status, ConnectionStatus::Connecting);
        assert!(state.url.is_none());
        assert!(state.metrics_port.is_none());
        assert!(state.requests.is_empty());
        assert!(state.filter.is_empty());
        assert!(!state.filter_active);
        assert!(state.flash.is_none());
    }

    #[test]
    fn apply_url_event_sets_connected() {
        let mut state = AppState::new(3000, crate::mock::new_rules());
        state.apply_event(TunnelEvent::Url("https://test.trycloudflare.com".into()));
        assert_eq!(state.status, ConnectionStatus::Connected);
        assert_eq!(state.url.as_deref(), Some("https://test.trycloudflare.com"));
        assert!(state.connected_at.is_some());
    }

    #[test]
    fn apply_metrics_event() {
        let mut state = AppState::new(3000, crate::mock::new_rules());
        state.apply_event(TunnelEvent::Metrics(TunnelMetrics { total_requests: 42, request_errors: 1 }));
        assert_eq!(state.metrics.total_requests, 42);
        assert_eq!(state.metrics.request_errors, 1);
    }

    #[test]
    fn apply_disconnected_event() {
        let mut state = AppState::new(3000, crate::mock::new_rules());
        state.apply_event(TunnelEvent::Disconnected("process exited".into()));
        assert_eq!(state.status, ConnectionStatus::Disconnected("process exited".into()));
    }

    #[test]
    fn uptime_zero_when_not_connected() {
        let state = AppState::new(3000, crate::mock::new_rules());
        assert_eq!(state.uptime(), Duration::ZERO);
    }

    #[test]
    fn format_duration_works() {
        assert_eq!(format_duration(Duration::from_secs(0)), "00:00:00");
        assert_eq!(format_duration(Duration::from_secs(61)), "00:01:01");
        assert_eq!(format_duration(Duration::from_secs(3661)), "01:01:01");
    }

    #[test]
    fn apply_http_request_event() {
        let mut state = AppState::new(3000, crate::mock::new_rules());
        state.apply_event(TunnelEvent::HttpRequest(HttpRequest {
            method: "GET".into(), path: "/hello".into(), status: 200,
            duration: Duration::from_millis(42),
            remote_ip: Some("1.2.3.4".into()), country: Some("US".into()),
            user_agent: Some("curl/8.1".into()),
            request_headers: Vec::new(), response_headers: Vec::new(),
            request_size: 0, response_size: 1024,
            request_body: None, response_body: Some("{\"ok\":true}".into()),
            is_websocket: false, ws_frames: Vec::new(), is_mock: false, timestamp: chrono::Local::now(),
        }));
        assert_eq!(state.requests.len(), 1);
        assert_eq!(state.requests[0].method, "GET");
        assert_eq!(state.requests[0].status, 200);
    }

    #[test]
    fn requests_capped_at_max() {
        let mut state = AppState::new(3000, crate::mock::new_rules());
        for i in 0..(MAX_REQUESTS + 10) {
            state.apply_event(TunnelEvent::HttpRequest(HttpRequest {
                method: "GET".into(), path: format!("/{}", i), status: 200,
                duration: Duration::from_millis(1),
                remote_ip: None, country: None, user_agent: None,
                request_headers: Vec::new(), response_headers: Vec::new(),
                request_size: 0, response_size: 0,
                request_body: None, response_body: None,
                is_websocket: false, ws_frames: Vec::new(), is_mock: false, timestamp: chrono::Local::now(),
            }));
        }
        assert_eq!(state.requests.len(), MAX_REQUESTS);
        assert_eq!(state.requests[0].path, "/10");
    }

    #[test]
    fn apply_multiple_events_in_sequence() {
        let mut state = AppState::new(8080, crate::mock::new_rules());
        state.apply_event(TunnelEvent::Connecting);
        assert_eq!(state.status, ConnectionStatus::Connecting);
        state.apply_event(TunnelEvent::Version("2024.1.5".into()));
        assert_eq!(state.version.as_deref(), Some("2024.1.5"));
        state.apply_event(TunnelEvent::MetricsPort(9999));
        assert_eq!(state.metrics_port, Some(9999));
        state.apply_event(TunnelEvent::EdgeLocation("lax".into()));
        assert_eq!(state.edge_location.as_deref(), Some("lax"));
        state.apply_event(TunnelEvent::Url("https://x.trycloudflare.com".into()));
        assert_eq!(state.status, ConnectionStatus::Connected);
    }

    #[test]
    fn format_request_duration_ms() {
        assert_eq!(format_request_duration(Duration::from_millis(42)), "42ms");
        assert_eq!(format_request_duration(Duration::from_millis(999)), "999ms");
    }

    #[test]
    fn format_request_duration_secs() {
        assert_eq!(format_request_duration(Duration::from_millis(1500)), "1.5s");
    }

    fn make_request(method: &str, path: &str, status: u16) -> HttpRequest {
        HttpRequest {
            method: method.into(), path: path.into(), status,
            duration: Duration::from_millis(10),
            remote_ip: None, country: None, user_agent: None,
            request_headers: Vec::new(), response_headers: Vec::new(),
            request_size: 0, response_size: 0,
            request_body: None, response_body: None,
            is_websocket: false, ws_frames: Vec::new(), is_mock: false, timestamp: chrono::Local::now(),
        }
    }

    #[test]
    fn filter_by_path() {
        let mut state = AppState::new(3000, crate::mock::new_rules());
        state.apply_event(TunnelEvent::HttpRequest(make_request("GET", "/api/users", 200)));
        state.apply_event(TunnelEvent::HttpRequest(make_request("POST", "/api/login", 200)));
        state.apply_event(TunnelEvent::HttpRequest(make_request("GET", "/health", 200)));
        state.filter = "api".into();
        assert_eq!(state.filtered_indices().len(), 2);
    }

    #[test]
    fn filter_by_status() {
        let mut state = AppState::new(3000, crate::mock::new_rules());
        state.apply_event(TunnelEvent::HttpRequest(make_request("GET", "/a", 200)));
        state.apply_event(TunnelEvent::HttpRequest(make_request("GET", "/b", 404)));
        state.apply_event(TunnelEvent::HttpRequest(make_request("GET", "/c", 500)));
        state.filter = "404".into();
        assert_eq!(state.filtered_indices().len(), 1);
    }

    #[test]
    fn filter_by_method() {
        let mut state = AppState::new(3000, crate::mock::new_rules());
        state.apply_event(TunnelEvent::HttpRequest(make_request("GET", "/a", 200)));
        state.apply_event(TunnelEvent::HttpRequest(make_request("POST", "/b", 200)));
        state.apply_event(TunnelEvent::HttpRequest(make_request("GET", "/c", 200)));
        state.filter = "post".into();
        assert_eq!(state.filtered_indices().len(), 1);
    }

    #[test]
    fn empty_filter_returns_all() {
        let mut state = AppState::new(3000, crate::mock::new_rules());
        state.apply_event(TunnelEvent::HttpRequest(make_request("GET", "/a", 200)));
        state.apply_event(TunnelEvent::HttpRequest(make_request("GET", "/b", 200)));
        state.filter = String::new();
        assert_eq!(state.filtered_indices().len(), 2);
    }

    #[test]
    fn curl_generation() {
        use crate::event::HeaderPair;
        let req = HttpRequest {
            method: "POST".into(), path: "/api/webhook".into(), status: 200,
            duration: Duration::from_millis(10),
            remote_ip: None, country: None, user_agent: None,
            request_headers: vec![
                HeaderPair { name: "content-type".into(), value: "application/json".into() },
                HeaderPair { name: "x-custom".into(), value: "test".into() },
                HeaderPair { name: "cf-connecting-ip".into(), value: "1.2.3.4".into() },
            ],
            response_headers: Vec::new(),
            request_size: 13, response_size: 0,
            request_body: Some("{\"hello\":true}".into()), response_body: None,
            is_websocket: false, ws_frames: Vec::new(), is_mock: false, timestamp: chrono::Local::now(),
        };
        let curl = req.to_curl("https://example.trycloudflare.com");
        assert!(curl.contains("curl -X POST"));
        assert!(curl.contains("content-type: application/json"));
        assert!(curl.contains("x-custom: test"));
        assert!(!curl.contains("cf-connecting-ip"));
        assert!(curl.contains("/api/webhook"));
        assert!(curl.contains("{\"hello\":true}"));
    }

    #[test]
    fn flash_message_sets_and_exists() {
        let mut state = AppState::new(3000, crate::mock::new_rules());
        assert!(state.flash.is_none());
        state.set_flash("test message");
        assert!(state.flash.is_some());
        assert_eq!(state.flash.as_ref().unwrap().0, "test message");
    }

    #[test]
    fn websocket_request_in_state() {
        let mut state = AppState::new(3000, crate::mock::new_rules());
        state.apply_event(TunnelEvent::HttpRequest(HttpRequest {
            method: "GET".into(), path: "/ws".into(), status: 101,
            duration: Duration::from_millis(0),
            remote_ip: None, country: None, user_agent: None,
            request_headers: Vec::new(), response_headers: Vec::new(),
            request_size: 0, response_size: 0,
            request_body: None, response_body: None,
            is_websocket: true, ws_frames: Vec::new(), is_mock: false, timestamp: chrono::Local::now(),
        }));
        assert_eq!(state.requests.len(), 1);
        assert!(state.requests[0].is_websocket);
    }

    #[test]
    fn websocket_frames_appended_to_request() {
        use crate::event::{WebSocketFrame, WsDirection, WsOpcode};
        use std::time::Instant;

        let mut state = AppState::new(3000, crate::mock::new_rules());
        state.apply_event(TunnelEvent::HttpRequest(HttpRequest {
            method: "GET".into(), path: "/ws".into(), status: 101,
            duration: Duration::from_millis(0),
            remote_ip: None, country: None, user_agent: None,
            request_headers: Vec::new(), response_headers: Vec::new(),
            request_size: 0, response_size: 0,
            request_body: None, response_body: None,
            is_websocket: true, ws_frames: Vec::new(), is_mock: false, timestamp: chrono::Local::now(),
        }));
        state.apply_event(TunnelEvent::WebSocketFrame(WebSocketFrame {
            direction: WsDirection::ClientToServer,
            opcode: WsOpcode::Text,
            payload_preview: Some("hello".into()),
            payload_size: 5,
            timestamp: Instant::now(),
        }));
        assert_eq!(state.requests[0].ws_frames.len(), 1);
        assert!(matches!(state.requests[0].ws_frames[0].direction, WsDirection::ClientToServer));
    }

    #[test]
    fn mark_and_unmark() {
        let mut state = AppState::new(3000, crate::mock::new_rules());
        state.apply_event(TunnelEvent::HttpRequest(make_request("GET", "/a", 200)));
        state.selected = Some(0);
        state.marked = Some(0);
        assert_eq!(state.marked, Some(0));
        state.marked = None;
        assert!(state.marked.is_none());
    }

    #[test]
    fn selection_shifts_when_new_request_arrives() {
        let mut state = AppState::new(3000, crate::mock::new_rules());
        // Add 3 requests: deque = [A, B, C], reversed display = [C, B, A]
        state.apply_event(TunnelEvent::HttpRequest(make_request("GET", "/a", 200)));
        state.apply_event(TunnelEvent::HttpRequest(make_request("GET", "/b", 200)));
        state.apply_event(TunnelEvent::HttpRequest(make_request("GET", "/c", 200)));

        // Select row 1 (= B in reversed view), mark row 2 (= A)
        state.selected = Some(1);
        state.marked = Some(2);

        // New request D arrives: deque = [A, B, C, D], reversed = [D, C, B, A]
        // Selection should shift to keep pointing at B (now row 2) and A (now row 3)
        state.apply_event(TunnelEvent::HttpRequest(make_request("GET", "/d", 200)));
        assert_eq!(state.selected, Some(2));
        assert_eq!(state.marked, Some(3));

        // Verify we still resolve to the same request
        let indices = state.filtered_indices();
        let selected_req = indices.get(2).and_then(|&ri| state.requests.iter().rev().nth(ri));
        assert_eq!(selected_req.unwrap().path, "/b");
    }

    #[test]
    fn parse_ua_browsers() {
        assert_eq!(parse_ua("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.6099.129 Safari/537.36"), "Chrome 120");
        assert_eq!(parse_ua("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 Edg/120.0.2210.91"), "Edge 120");
        assert_eq!(parse_ua("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_2_1) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.2 Safari/605.1.15"), "Safari 17");
        assert_eq!(parse_ua("Mozilla/5.0 (X11; Linux x86_64; rv:121.0) Gecko/20100101 Firefox/121.0"), "Firefox 121");
    }

    #[test]
    fn parse_ua_tools() {
        assert_eq!(parse_ua("curl/8.4.0"), "curl/8.4");
        assert_eq!(parse_ua("python-requests/2.31.0"), "python-requests");
        assert_eq!(parse_ua("PostmanRuntime/7.36.0"), "Postman");
    }

    #[test]
    fn parse_ua_bots() {
        assert_eq!(parse_ua("Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)"), "Googlebot");
        assert_eq!(parse_ua("Slackbot-LinkExpanding 1.0"), "Slack");
    }
}
