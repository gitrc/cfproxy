use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::event::{HttpRequest, TunnelEvent};
use crate::metrics::TunnelMetrics;
use crate::mock::MockRules;
use crate::settings::Settings;

pub const MAX_REQUESTS: usize = 10_000;
const SPARKLINE_WIDTH: usize = 60;

#[derive(Debug, PartialEq)]
pub enum ConnectionStatus {
    Connecting,
    Connected,
    Disconnected(String),
}

#[derive(Debug, PartialEq)]
pub enum ViewMode {
    List,
    Detail,
    Diff,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum DetailTab {
    Request,
    Response,
    Info,
    WebSocket,
}

impl DetailTab {
    pub fn all(has_ws: bool) -> Vec<DetailTab> {
        let mut tabs = vec![DetailTab::Request, DetailTab::Response, DetailTab::Info];
        if has_ws {
            tabs.push(DetailTab::WebSocket);
        }
        tabs
    }

    pub fn label(self) -> &'static str {
        match self {
            DetailTab::Request => "Request",
            DetailTab::Response => "Response",
            DetailTab::Info => "Info",
            DetailTab::WebSocket => "WS Frames",
        }
    }
}

pub struct RequestStats {
    pub total: u64,
    pub status_2xx: u64,
    pub status_3xx: u64,
    pub status_4xx: u64,
    pub status_5xx: u64,
    pub total_duration_ms: u64,
    pub rps_history: VecDeque<u64>,
    pub current_second_count: u64,
}

impl RequestStats {
    pub fn record(&mut self, status: u16, duration: Duration) {
        self.total += 1;
        self.current_second_count += 1;
        match status {
            200..=299 => self.status_2xx += 1,
            300..=399 => self.status_3xx += 1,
            400..=499 => self.status_4xx += 1,
            _ => self.status_5xx += 1,
        }
        self.total_duration_ms += duration.as_millis() as u64;
    }

    pub fn tick(&mut self) {
        self.rps_history.push_back(self.current_second_count);
        if self.rps_history.len() > SPARKLINE_WIDTH {
            self.rps_history.pop_front();
        }
        self.current_second_count = 0;
    }

    pub fn avg_ms(&self) -> u64 {
        if self.total == 0 { 0 } else { self.total_duration_ms / self.total }
    }

    pub fn rps_data(&self) -> Vec<u64> {
        let len = self.rps_history.len();
        let mut data = vec![0u64; SPARKLINE_WIDTH.saturating_sub(len)];
        data.extend(self.rps_history.iter());
        data
    }
}

impl Default for RequestStats {
    fn default() -> Self {
        Self {
            total: 0,
            status_2xx: 0,
            status_3xx: 0,
            status_4xx: 0,
            status_5xx: 0,
            total_duration_ms: 0,
            rps_history: VecDeque::with_capacity(SPARKLINE_WIDTH + 1),
            current_second_count: 0,
        }
    }
}

pub struct AppState {
    pub status: ConnectionStatus,
    pub url: Option<String>,
    pub port: u16,
    pub edge_location: Option<String>,
    pub version: Option<String>,
    pub connected_at: Option<Instant>,
    pub metrics: TunnelMetrics,
    pub metrics_port: Option<u16>,
    pub requests: VecDeque<HttpRequest>,
    pub stats: RequestStats,
    pub selected: Option<usize>,
    pub detail_scroll: u16,
    pub detail_tab: DetailTab,
    pub view: ViewMode,
    pub filter: String,
    pub filter_active: bool,
    pub flash: Option<(String, Instant)>,
    pub show_qr: bool,
    pub show_help: bool,
    pub marked: Option<usize>,
    pub mock_rules: MockRules,
    pub confirm_quit: bool,
    pub watch: Option<String>,
    pub show_settings: bool,
    pub settings: Settings,
    pub settings_field: usize,
    pub settings_inputs: Vec<String>,
}

impl AppState {
    pub fn new(port: u16, mock_rules: MockRules) -> Self {
        Self {
            status: ConnectionStatus::Connecting,
            url: None,
            port,
            edge_location: None,
            version: None,
            connected_at: None,
            metrics: TunnelMetrics::default(),
            metrics_port: None,
            requests: VecDeque::with_capacity(MAX_REQUESTS),
            stats: RequestStats::default(),
            selected: None,
            detail_scroll: 0,
            detail_tab: DetailTab::Request,
            view: ViewMode::List,
            filter: String::new(),
            filter_active: false,
            flash: None,
            show_qr: false,
            show_help: false,
            marked: None,
            mock_rules,
            confirm_quit: false,
            watch: None,
            show_settings: false,
            settings: Settings::load(),
            settings_field: 0,
            settings_inputs: vec![String::new(); 4],
        }
    }

    pub fn apply_event(&mut self, event: TunnelEvent) {
        match event {
            TunnelEvent::Connecting => {
                self.status = ConnectionStatus::Connecting;
            }
            TunnelEvent::Url(url) => {
                self.url = Some(url);
                self.status = ConnectionStatus::Connected;
                if self.connected_at.is_none() {
                    self.connected_at = Some(Instant::now());
                }
            }
            TunnelEvent::EdgeLocation(loc) => {
                self.edge_location = Some(loc);
            }
            TunnelEvent::Version(ver) => {
                self.version = Some(ver);
            }
            TunnelEvent::MetricsPort(port) => {
                self.metrics_port = Some(port);
            }
            TunnelEvent::Metrics(m) => {
                self.metrics = m;
            }
            TunnelEvent::HttpRequest(req) => {
                self.stats.record(req.status, req.duration);
                let watch_match = self.watch.as_ref().is_some_and(|pattern| {
                    let p = pattern.to_lowercase();
                    req.path.to_lowercase().contains(&p)
                        || req.method.to_lowercase().contains(&p)
                });

                // New request appears at position 0 in reversed view, shifting
                // existing items down. Adjust selection to track the same request.
                let shifts_filtered = if self.filter.is_empty() {
                    true
                } else {
                    let f = self.filter.to_lowercase();
                    req.path.to_lowercase().contains(&f)
                        || req.method.to_lowercase().contains(&f)
                        || req.status.to_string().contains(&f)
                        || req.user_agent.as_deref().unwrap_or("").to_lowercase().contains(&f)
                        || req.remote_ip.as_deref().unwrap_or("").contains(&f)
                };
                if shifts_filtered {
                    if let Some(ref mut sel) = self.selected {
                        *sel += 1;
                    }
                    if let Some(ref mut m) = self.marked {
                        *m += 1;
                    }
                }

                if self.requests.len() >= MAX_REQUESTS {
                    self.requests.pop_front();
                }
                self.requests.push_back(req);
                if watch_match {
                    self.selected = Some(0);
                    self.detail_scroll = 0;
                    self.detail_tab = DetailTab::Request;
                    self.view = ViewMode::Detail;
                }
            }
            TunnelEvent::WebSocketFrame(frame) => {
                const MAX_WS_FRAMES: usize = 10_000;
                if let Some(req) = self.requests.iter_mut().rev().find(|r| r.is_websocket) {
                    if req.ws_frames.len() >= MAX_WS_FRAMES {
                        req.ws_frames.remove(0);
                    }
                    req.ws_frames.push(frame);
                }
            }
            TunnelEvent::Disconnected(msg) => {
                self.status = ConnectionStatus::Disconnected(msg);
            }
        }
    }

    pub fn uptime(&self) -> Duration {
        self.connected_at
            .map(|t| t.elapsed())
            .unwrap_or(Duration::ZERO)
    }

    pub fn filtered_indices(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.requests.len()).collect();
        }
        let f = self.filter.to_lowercase();
        self.requests
            .iter()
            .rev()
            .enumerate()
            .filter(|(_, req)| {
                req.path.to_lowercase().contains(&f)
                    || req.method.to_lowercase().contains(&f)
                    || req.status.to_string().contains(&f)
                    || req.user_agent.as_deref().unwrap_or("").to_lowercase().contains(&f)
                    || req.remote_ip.as_deref().unwrap_or("").contains(&f)
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn set_flash(&mut self, msg: &str) {
        self.flash = Some((msg.to_string(), Instant::now()));
    }
}
