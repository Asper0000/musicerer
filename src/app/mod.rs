mod events;
mod helpers;
mod update;
mod view;

use crate::backend::vpn::{self, VpnInfo};
use crate::theme;
use crate::ui::{ItemStatus, LogLine, QueueItem, ThumbState, View};

use iced::{Subscription, Task, Theme};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Settings kept between runs. Unknown keys in older config files are ignored.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct AppConfig {
    output_dir: Option<String>,
    cookies_input: Option<String>,
    group_playlists: Option<bool>,
    skip_existing: Option<bool>,
    suppress_vpn_warning: Option<bool>,
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
    ToggleGroup(u64),
    ClearQueue,
    OutputDirChanged(String),
    BrowseOutputDir,
    OutputDirPicked(Option<PathBuf>),
    OpenOutputDir,
    CookiesChanged(String),
    BrowseCookies,
    CookiesPicked(Option<PathBuf>),
    ToggleGroupPlaylists(bool),
    ToggleSkipExisting(bool),
    StartQueue,
    StopQueue,
    SwitchView(View),
    ToggleLog,
    TogglePlatformsDialog,
    DismissVpnWarning,
    SuppressVpnWarning,
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
    /// Save playlist and album tracks into their own subfolder.
    pub(crate) group_playlists: bool,
    pub(crate) skip_existing: bool,
    pub(crate) suppress_vpn_warning: bool,
    /// Animation counter, advanced on every worker poll.
    pub(crate) tick: u32,
    pub(crate) queue: Vec<QueueItem>,
    pub(crate) logs: VecDeque<LogLine>,
    pub(crate) next_id: u64,
    pub(crate) downloading: bool,
    pub(crate) status: String,
    pub(crate) show_log: bool,
    /// Frames left to highlight the Log button (set when a user-facing error is logged).
    pub(crate) log_attention: u8,
    pub(crate) show_platforms: bool,
    pub(crate) startup_vpn_warning: Option<VpnInfo>,
    pub(crate) inbox: Arc<Mutex<Vec<crate::backend::WorkerEvent>>>,
    /// Raised by Stop to end the running download queue.
    pub(crate) stop_flag: Arc<AtomicBool>,
    /// Shared by every lookup in flight (playlist expansion, previews, Spotify matching).
    /// Stop raises it and installs a fresh one for lookups started afterwards.
    pub(crate) resolve_cancel: Arc<AtomicBool>,
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        let vpn_hit = vpn::detect();
        let cfg = load_config();
        let suppress_vpn_warning = cfg.suppress_vpn_warning.unwrap_or(false);
        let startup_vpn_warning = if suppress_vpn_warning {
            None
        } else if vpn_hit.detected {
            Some(vpn_hit)
        } else {
            None
        };

        let default_output = dirs::download_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Musicerer")
            .to_string_lossy()
            .to_string();

        let mut app = Self {
            current_view: View::Downloads,
            url_input: String::new(),
            output_dir: cfg.output_dir.unwrap_or(default_output),
            available_space: String::new(),
            cookies_input: cfg.cookies_input.unwrap_or_default(),
            group_playlists: cfg.group_playlists.unwrap_or(true),
            skip_existing: cfg.skip_existing.unwrap_or(false),
            suppress_vpn_warning: cfg.suppress_vpn_warning.unwrap_or(false),
            tick: 0,
            queue: Vec::new(),
            logs: VecDeque::new(),
            next_id: 1,
            downloading: false,
            status: "Ready".to_string(),
            show_log: false,
            log_attention: 0,
            show_platforms: false,
            startup_vpn_warning,
            inbox: Arc::new(Mutex::new(Vec::new())),
            stop_flag: Arc::new(AtomicBool::new(false)),
            resolve_cancel: Arc::new(AtomicBool::new(false)),
        };

        app.update_disk_space();

        (app, Task::none())
    }

    pub fn theme(&self) -> Theme {
        theme::app_theme()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        if self.is_active() {
            iced::time::every(Duration::from_millis(120)).map(|_| Message::PollWorker)
        } else {
            Subscription::none()
        }
    }

    fn is_active(&self) -> bool {
        self.downloading
            || self.queue.iter().any(|i| {
                i.resolving
                    || i.tracks()
                        .any(|t| matches!(t.thumbnail, ThumbState::Loading))
            })
    }

    /// Every downloadable track in the queue: playlist children included.
    pub(crate) fn tracks(&self) -> impl Iterator<Item = &QueueItem> {
        self.queue.iter().flat_map(|item| item.tracks())
    }

    pub(crate) fn find_item_mut(&mut self, id: u64) -> Option<&mut QueueItem> {
        self.queue.iter_mut().find_map(|item| item.find_mut(id))
    }

    /// Average completion across all tracks; finished tracks count as 100% whatever the outcome.
    pub(crate) fn overall_progress(&self) -> f32 {
        if !self.downloading {
            return 0.0;
        }
        let (sum, count) = self.tracks().fold((0.0f32, 0usize), |(sum, count), t| {
            let p = match t.status {
                ItemStatus::Done
                | ItemStatus::Failed
                | ItemStatus::Unavailable
                | ItemStatus::DrmSkipped
                | ItemStatus::Stopped => 1.0,
                _ => t.progress,
            };
            (sum + p, count + 1)
        });
        if count == 0 {
            0.0
        } else {
            sum / count as f32
        }
    }

    pub(crate) fn cookies_path(&self) -> Option<PathBuf> {
        let s = self.cookies_input.trim();
        if s.is_empty() {
            None
        } else {
            Some(PathBuf::from(s))
        }
    }

    fn save_config(&self) {
        let Some(path) = config_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let cfg = AppConfig {
            output_dir: Some(self.output_dir.clone()),
            cookies_input: Some(self.cookies_input.clone()),
            group_playlists: Some(self.group_playlists),
            skip_existing: Some(self.skip_existing),
            suppress_vpn_warning: Some(self.suppress_vpn_warning),
        };
        if let Ok(json) = serde_json::to_string_pretty(&cfg) {
            let _ = std::fs::write(path, json);
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
            let fmt = |b: f64| {
                if b >= tb {
                    format!("{:.1} TB", b / tb)
                } else {
                    format!("{:.1} GB", b / gb)
                }
            };
            self.available_space = format!("{} / {}", fmt(avail), fmt(total));
        } else {
            self.available_space = "Unknown".to_string();
        }
    }
}
