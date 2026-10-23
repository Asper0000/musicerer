mod helpers;
mod update;
mod view;

pub use helpers::guess_name_from_url;

use crate::backend::vpn::{self, VpnInfo};
use crate::theme;
use crate::ui::{LogLine, QueueItem, View};

use iced::{Subscription, Task, Theme};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct AppConfig {
    output_dir: Option<String>,
    cookies_input: Option<String>,
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("Musicerer").join("config.json"))
}

fn load_config() -> AppConfig {
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
pub enum Message {
    UrlChanged(String),
    AddUrl,
    RemoveTrack(u64),
    ClearQueue,
    OutputDirChanged(String),
    BrowseOutputDir,
    OutputDirPicked(Option<PathBuf>),
    OpenOutputDir,
    CookiesChanged(String),
    BrowseCookies,
    CookiesPicked(Option<PathBuf>),
    StartQueue,
    StopQueue,
    SwitchView(View),
    ToggleLog,
    TogglePlatformsDialog,
    DismissVpnWarning,
    PollWorker,
    WindowDragged,
    WindowMinimize,
    WindowMaximize,
    WindowClose,
    WindowResize(iced::window::Direction),
}

pub struct App {
    pub(crate) current_view: View,
    pub(crate) url_input: String,
    pub(crate) output_dir: String,
    pub(crate) available_space: String,
    pub(crate) cookies_input: String,
    pub(crate) queue: Vec<QueueItem>,
    pub(crate) logs: VecDeque<LogLine>,
    pub(crate) next_id: u64,
    pub(crate) downloading: bool,
    pub(crate) current_progress: f32,
    pub(crate) status: String,
    pub(crate) show_log: bool,
    pub(crate) show_platforms: bool,
    pub(crate) startup_vpn_warning: Option<VpnInfo>,
    pub(crate) inbox: Arc<Mutex<Vec<crate::backend::WorkerEvent>>>,
    pub(crate) stop_flag: Arc<AtomicBool>,
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        let vpn_hit = vpn::detect();
        let startup_vpn_warning = if vpn_hit.detected { Some(vpn_hit) } else { None };

        let cfg = load_config();

        let default_output = dirs::download_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Musicerer")
            .to_string_lossy()
            .to_string();

        let output_dir = cfg.output_dir.unwrap_or(default_output);
        let cookies_input = cfg.cookies_input.unwrap_or_default();

        let mut app = Self {
            current_view: View::Downloads,
            url_input: String::new(),
            output_dir,
            available_space: String::new(),
            cookies_input,
            queue: Vec::new(),
            logs: VecDeque::new(),
            next_id: 1,
            downloading: false,
            current_progress: 0.0,
            status: "Ready".to_string(),
            show_log: false,
            show_platforms: false,
            startup_vpn_warning,
            inbox: Arc::new(Mutex::new(Vec::new())),
            stop_flag: Arc::new(AtomicBool::new(false)),
        };

        app.update_disk_space();

        (app, Task::none())
    }

    pub fn theme(&self) -> Theme {
        theme::app_theme()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        iced::time::every(Duration::from_millis(120)).map(|_| Message::PollWorker)
    }

    pub(crate) fn cookies_path(&self) -> Option<PathBuf> {
        let s = self.cookies_input.trim();
        if s.is_empty() { None } else { Some(PathBuf::from(s)) }
    }

    fn save_config(&self) {
        if let Some(path) = config_path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let cfg = AppConfig {
                output_dir: Some(self.output_dir.clone()),
                cookies_input: Some(self.cookies_input.clone()),
            };
            if let Ok(json) = serde_json::to_string_pretty(&cfg) {
                let _ = std::fs::write(path, json);
            }
        }
    }

    fn update_disk_space(&mut self) {
        let disks = sysinfo::Disks::new_with_refreshed_list();
        let path = std::path::Path::new(&self.output_dir);
        let mut best_disk = None;
        let mut max_len = 0;
        for disk in &disks {
            if path.starts_with(disk.mount_point()) {
                let len = disk.mount_point().as_os_str().len();
                if len > max_len {
                    max_len = len;
                    best_disk = Some(disk);
                }
            }
        }
        if let Some(disk) = best_disk.or_else(|| disks.iter().next()) {
            let avail = disk.available_space() as f64;
            let total = disk.total_space() as f64;
            let tb = 1024.0 * 1024.0 * 1024.0 * 1024.0;
            let gb = 1024.0 * 1024.0 * 1024.0;
            let fmt = |b: f64| if b >= tb { format!("{:.1} TB", b / tb) } else { format!("{:.1} GB", b / gb) };
            self.available_space = format!("{} / {}", fmt(avail), fmt(total));
        } else {
            self.available_space = "Unknown".to_string();
        }
    }
}
