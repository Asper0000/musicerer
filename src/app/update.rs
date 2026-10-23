use super::{guess_name_from_url, App, Message};
use crate::backend::{
    detect_source, normalize_url, spawn_preview, spawn_worker, LogLevel, Source, TrackResult,
    WorkerEvent,
};
use crate::ui::{ItemStatus, QueueItem, ThumbState, UiLogLevel, View};

use iced::Task;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

impl App {
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WindowDragged => {
                return iced::window::latest().and_then(|id| iced::window::drag(id));
            }
            Message::WindowMinimize => {
                return iced::window::latest().and_then(|id| iced::window::minimize(id, true));
            }
            Message::WindowMaximize => {
                return iced::window::latest().and_then(|id| iced::window::toggle_maximize(id));
            }
            Message::WindowClose => {
                return iced::window::latest().and_then(|id| iced::window::close(id));
            }
            Message::WindowResize(dir) => {
                return iced::window::latest().and_then(move |id| iced::window::drag_resize(id, dir));
            }
            Message::OpenOutputDir => {
                let path = std::path::Path::new(&self.output_dir);
                let _ = std::fs::create_dir_all(path);
                #[cfg(target_os = "windows")]
                let _ = std::process::Command::new("explorer").arg(path).spawn();
                #[cfg(target_os = "macos")]
                let _ = std::process::Command::new("open").arg(path).spawn();
                #[cfg(target_os = "linux")]
                let _ = std::process::Command::new("xdg-open").arg(path).spawn();
            }
            Message::TogglePlatformsDialog => self.show_platforms = !self.show_platforms,
            Message::SwitchView(v) => self.current_view = v,
            Message::ToggleLog => self.show_log = !self.show_log,
            Message::DismissVpnWarning => self.startup_vpn_warning = None,
            Message::UrlChanged(v) => self.url_input = v,
            Message::AddUrl => {
                if self.downloading { return Task::none(); }
                let url = normalize_url(&self.url_input);
                if url.is_empty() { return Task::none(); }
                let source = detect_source(&url);
                if source == Source::Unsupported {
                    self.push_log(
                        UiLogLevel::Warn,
                        "Only SoundCloud and YouTube links are supported.",
                    );
                    return Task::none();
                }
                let id = self.next_id;
                self.next_id += 1;
                let display_name = guess_name_from_url(&url, source);
                let preview_url = url.clone();
                self.queue.push(QueueItem {
                    id,
                    url,
                    source,
                    status: ItemStatus::Pending,
                    progress: 0.0,
                    display_name,
                    thumbnail: ThumbState::Loading,
                });
                spawn_preview(id, preview_url, self.cookies_path(), self.inbox.clone());
                self.url_input.clear();
            }
            Message::RemoveTrack(id) => {
                if !self.downloading { self.queue.retain(|i| i.id != id); }
            }
            Message::ClearQueue => {
                if !self.downloading { self.queue.clear(); }
            }
            Message::OutputDirChanged(v) => {
                self.output_dir = v;
                self.update_disk_space();
                self.save_config();
            }
            Message::BrowseOutputDir => {
                return Task::perform(
                    async { rfd::FileDialog::new().pick_folder() },
                    Message::OutputDirPicked,
                );
            }
            Message::OutputDirPicked(p) => {
                if let Some(p) = p {
                    self.output_dir = p.to_string_lossy().to_string();
                    self.update_disk_space();
                    self.save_config();
                }
            }
            Message::CookiesChanged(v) => {
                self.cookies_input = v;
                self.save_config();
            }
            Message::BrowseCookies => {
                return Task::perform(
                    async {
                        rfd::FileDialog::new()
                            .add_filter("Cookies", &["txt"])
                            .pick_file()
                    },
                    Message::CookiesPicked,
                );
            }
            Message::CookiesPicked(p) => {
                if let Some(p) = p {
                    self.cookies_input = p.to_string_lossy().to_string();
                    self.save_config();
                }
            }
            Message::StartQueue => {
                if self.downloading || self.queue.is_empty() { return Task::none(); }
                let output_dir = PathBuf::from(self.output_dir.trim());
                if output_dir.as_os_str().is_empty() {
                    self.push_log(UiLogLevel::Error, "Choose an output folder first.");
                    self.current_view = View::Settings;
                    return Task::none();
                }
                if let Err(e) = std::fs::create_dir_all(&output_dir) {
                    self.push_log(
                        UiLogLevel::Error,
                        format!("Cannot create output folder: {e}"),
                    );
                    return Task::none();
                }
                let cookies = if self.cookies_input.trim().is_empty() {
                    None
                } else {
                    let p = PathBuf::from(self.cookies_input.trim());
                    if !p.exists() {
                        self.push_log(
                            UiLogLevel::Error,
                            format!("Cookies file not found: {}", p.display()),
                        );
                        return Task::none();
                    }
                    Some(p)
                };
                let items = self
                    .queue
                    .iter()
                    .map(|i| (i.id, i.url.clone()))
                    .collect::<Vec<_>>();
                for i in &mut self.queue {
                    i.status = ItemStatus::Pending;
                    i.progress = 0.0;
                }
                self.downloading = true;
                self.current_progress = 0.0;
                self.stop_flag.store(false, Ordering::SeqCst);
                self.logs.clear();
                self.push_log(UiLogLevel::Info, "Queue started.");
                self.status = "Startingâ€¦".to_string();
                spawn_worker(
                    items,
                    output_dir,
                    cookies,
                    self.inbox.clone(),
                    self.stop_flag.clone(),
                );
            }
            Message::StopQueue => {
                if self.downloading {
                    self.stop_flag.store(true, Ordering::SeqCst);
                    self.status = "Stopping after current trackâ€¦".to_string();
                    self.push_log(UiLogLevel::Warn, "Stop requested.");
                }
            }
            Message::PollWorker => {
                let events = {
                    let mut g = self.inbox.lock().unwrap_or_else(|e| e.into_inner());
                    std::mem::take(&mut *g)
                };
                for e in events { self.handle_worker_event(e); }
            }
        }
        Task::none()
    }

    fn handle_worker_event(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::TrackStarted { index } => {
                if let Some(i) = self.queue.get_mut(index) {
                    i.status = ItemStatus::Downloading;
                    i.progress = 0.0;
                }
                self.current_progress = 0.0;
                self.status = format!("Downloading {}/{}â€¦", index + 1, self.queue.len());
            }
            WorkerEvent::TrackTitle { index, title } => {
                if let Some(i) = self.queue.get_mut(index) { i.display_name = title; }
            }
            WorkerEvent::Progress { index, value } => {
                if let Some(i) = self.queue.get_mut(index) { i.progress = value; }
                self.current_progress = value;
            }
            WorkerEvent::Thumbnail { index, width, height, rgba } => {
                if let Some(i) = self.queue.get_mut(index) {
                    i.thumbnail = ThumbState::Ready(iced::widget::image::Handle::from_rgba(
                        width, height, rgba,
                    ));
                }
            }
            WorkerEvent::Log { level, text } => {
                let lvl = match level {
                    LogLevel::Info => UiLogLevel::Info,
                    LogLevel::Warn => UiLogLevel::Warn,
                    LogLevel::Error => UiLogLevel::Error,
                };
                self.push_log(lvl, text);
            }
            WorkerEvent::TrackFinished { index, result } => {
                if let Some(i) = self.queue.get_mut(index) {
                    i.status = match result {
                        TrackResult::Ok => { i.progress = 1.0; ItemStatus::Done }
                        TrackResult::Drm => ItemStatus::DrmSkipped,
                        TrackResult::Error => ItemStatus::Failed,
                        TrackResult::Stopped => ItemStatus::Stopped,
                    };
                }
            }
            WorkerEvent::QueueFinished { succeeded, failed, drm, stopped } => {
                self.downloading = false;
                self.current_progress = 0.0;
                self.status = if stopped {
                    format!("Stopped â€” {succeeded} succeeded, {failed} failed, {drm} DRM-skipped")
                } else {
                    format!("Done â€” {succeeded} succeeded, {failed} failed, {drm} DRM-skipped")
                };
                self.push_log(UiLogLevel::Info, self.status.clone());
                self.update_disk_space();
            }
            WorkerEvent::PreviewReady { id, title, thumb } => {
                if let Some(i) = self.queue.iter_mut().find(|i| i.id == id) {
                    i.display_name = title;
                    i.thumbnail = match thumb {
                        Some((w, h, rgba)) => ThumbState::Ready(
                            iced::widget::image::Handle::from_rgba(w, h, rgba),
                        ),
                        None => ThumbState::Failed,
                    };
                }
            }
            WorkerEvent::PreviewFailed { id, error } => {
                if let Some(i) = self.queue.iter_mut().find(|i| i.id == id) {
                    i.thumbnail = ThumbState::Failed;
                    if i.display_name == "Fetching titleâ€¦" {
                        i.display_name = "Unknown video".to_string();
                    }
                }
                self.push_log(UiLogLevel::Warn, format!("Preview failed: {error}"));
            }
        }
    }

    fn push_log(&mut self, level: UiLogLevel, text: impl Into<String>) {
        self.logs.push_back(crate::ui::LogLine { level, text: text.into() });
        while self.logs.len() > 400 { self.logs.pop_front(); }
    }
}
