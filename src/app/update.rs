use super::helpers::{fallback_name, guess_name_from_url, FETCHING_TITLE, READING_PLAYLIST};
use super::{App, Message};
use crate::backend::{
    deezer, detect_source, is_playlist_url, itunes, normalize_url, spawn_preview, spawn_resolve,
    spawn_worker, spotify, updater, Source,
};
use crate::ui::{ItemStatus, QueueItem, ThumbState, UiLogLevel, View};

use iced::Task;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn open_with_system_file_manager(path: &Path) {
    // Some application directories are created lazily. Create the requested
    // directory when necessary so the file manager has a real target to open.
    if !path.exists() {
        let _ = std::fs::create_dir_all(path);
    }

    #[cfg(target_os = "windows")]
    {
        use std::ffi::c_void;
        use std::os::windows::ffi::OsStrExt;

        #[link(name = "shell32")]
        unsafe extern "system" {
            fn ShellExecuteW(
                window: *mut c_void,
                operation: *const u16,
                file: *const u16,
                parameters: *const u16,
                directory: *const u16,
                show_command: i32,
            ) -> *mut c_void;
        }

        let operation: Vec<u16> = std::ffi::OsStr::new("open")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let target: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // ShellExecute delegates to the Windows shell's registered folder
        // handler instead of hard-coding a particular file-manager process.
        unsafe {
            let _ = ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            );
        }
    }

    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

impl App {
    pub fn open_log_window(&mut self) -> Task<Message> {
        if self.log_window_id.is_none() {
            let icon = iced::window::icon::from_file_data(include_bytes!("../../assets/icon.ico"), None).ok();
            let settings = iced::window::Settings {
                size: iced::Size::new(600.0, 400.0),
                min_size: Some(iced::Size::new(400.0, 200.0)),
                decorations: false,
                transparent: true,
                icon,
                ..Default::default()
            };
            let (id, task) = iced::window::open(settings);
            self.log_window_id = Some(id);
            return task.map(|_| Message::Noop);
        }
        Task::none()
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WindowDragged(id) => {
                return iced::window::drag(id).map(|_: ()| Message::Noop);
            }
            Message::WindowMinimize(id) => {
                return iced::window::minimize(id, true).map(|_: ()| Message::Noop);
            }
            Message::WindowMaximize(id) => {
                return iced::window::toggle_maximize(id).map(|_: ()| Message::Noop);
            }
            Message::WindowClose(id) => {
                if Some(id) == self.main_window_id {
                    self.save_config();
                    return iced::exit();
                } else {
                    self.log_window_id = None;
                    return iced::window::close(id).map(|_: ()| Message::Noop);
                }
            }
            Message::WindowResize(id, dir) => {
                return iced::window::drag_resize(id, dir).map(|_: ()| Message::Noop);
            }
            Message::WindowEvent(id, iced::window::Event::Closed) => {
                if Some(id) == self.log_window_id {
                    self.log_window_id = None;
                } else if Some(id) == self.main_window_id {
                    return iced::exit();
                }
            }
            Message::WindowEvent(_, _) => {}
            Message::OpenLogWindow => {
                return self.open_log_window();
            }
            Message::CloseLogWindow => {
                if let Some(id) = self.log_window_id.take() {
                    return iced::window::close(id).map(|_: ()| Message::Noop);
                }
            }
            Message::LogWindowOpened(id) => self.log_window_id = Some(id),
            Message::MainWindowOpened(id) => self.main_window_id = Some(id),
            Message::OpenOutputDir => self.open_output_dir(),
            Message::OpenInstallationDirectory(path) => open_with_system_file_manager(&path),
            Message::TogglePlatformsDialog => self.show_platforms = !self.show_platforms,
            Message::ToggleToolInfoDialog => self.show_tool_info_dialog = !self.show_tool_info_dialog,
            Message::SwitchView(v) => {
                if self.current_view == View::Settings && v != View::Settings {
                    self.save_config();
                }
                if v == View::Settings {
                    self.update_disk_space();
                }
                self.current_view = v;
            }

            Message::DismissVpnWarning => self.startup_vpn_warning = None,
            Message::SuppressVpnWarning => {
                self.suppress_vpn_warning = true;
                self.startup_vpn_warning = None;
                self.save_config();
            }
            Message::UrlChanged(v) => self.url_input = v,
            Message::AddUrl => {
                self.add_url();
                if self.needs_log_window {
                    self.needs_log_window = false;
                    return self.open_log_window();
                }
            }
            Message::RemoveTrack(id) => self.remove_track(id),
            Message::ToggleGroup(id) => {
                if let Some(group) = self
                    .queue
                    .iter_mut()
                    .find(|i| i.id == id)
                    .and_then(|i| i.group.as_mut())
                {
                    group.expanded = !group.expanded;
                }
            }
            Message::ClearQueue => {
                if !self.downloading {
                    self.queue.clear();
                }
            }
            Message::OutputDirChanged(v) => {
                self.output_dir = v;
            }
            Message::BrowseOutputDir => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .pick_folder()
                            .await
                            .map(|handle| handle.path().to_path_buf())
                    },
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
            }
            Message::BrowseCookies => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .add_filter("Cookies", &["txt"])
                            .pick_file()
                            .await
                            .map(|handle| handle.path().to_path_buf())
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
            Message::ToggleGroupPlaylists(v) => {
                self.group_playlists = v;
                self.save_config();
            }
            Message::ToggleSkipExisting(v) => {
                self.skip_existing = v;
                self.save_config();
            }
            Message::StartQueue => {
                self.save_config();
                self.start_queue();
            }
            Message::StopQueue => self.stop_all(),
            Message::CheckUpdates => updater::spawn_check_all(&self.updater, false),
            Message::AskAppUpdate => {
                if !(self.downloading || self.is_loading()) {
                    self.confirm_app_update = true;
                }
            }
            Message::CancelAppUpdate => self.confirm_app_update = false,
            Message::InstallUpdate(comp) => {
                self.confirm_app_update = false;
                if self.downloading || self.is_loading() {
                    self.push_log(
                        UiLogLevel::Warn,
                        "Finish or stop the current downloads before updating.",
                    );
                } else if !updater::is_busy(&self.updater) {
                    updater::spawn_install(&self.updater, comp);
                }
            }
            Message::InstallExternalTools => {
                if self.downloading || self.is_loading() {
                    self.push_log(
                        UiLogLevel::Warn,
                        "Finish or stop the current downloads before installing tools.",
                    );
                } else if !updater::is_busy(&self.updater) {
                    updater::spawn_install_external(&self.updater);
                }
            }
            Message::InstallManagedTool(comp) => {
                if self.downloading || self.is_loading() {
                    self.push_log(UiLogLevel::Warn, "Finish or stop the current downloads before installing tools.");
                } else if updater::can_install_managed(&self.updater, comp) {
                    updater::spawn_install_managed(&self.updater, comp);
                }
            }
            Message::ReinstallManagedTool(comp) => {
                if self.downloading || self.is_loading() {
                    self.push_log(UiLogLevel::Warn, "Finish or stop the current downloads before reinstalling tools.");
                } else if updater::can_reinstall_tool(&self.updater, comp) {
                    updater::spawn_reinstall_tool(&self.updater, comp);
                }
            }
            Message::UninstallManagedTool(comp) => {
                if self.downloading || self.is_loading() {
                    self.push_log(UiLogLevel::Warn, "Finish or stop the current downloads before removing tools.");
                } else if !updater::is_busy(&self.updater) {
                    updater::spawn_uninstall_tool(&self.updater, comp);
                }
            }
            Message::AskAppReinstall => {
                if !(self.downloading || self.is_loading() || updater::is_busy(&self.updater))
                    && updater::can_reinstall_app(&self.updater)
                {
                    self.confirm_app_reinstall = true;
                }
            }
            Message::CancelAppReinstall => self.confirm_app_reinstall = false,
            Message::InstallAppReinstall => {
                self.confirm_app_reinstall = false;
                if self.downloading || self.is_loading() {
                    self.push_log(UiLogLevel::Warn, "Finish or stop the current downloads before reinstalling Musicerer.");
                } else if updater::can_reinstall_app(&self.updater) {
                    updater::spawn_reinstall_app(&self.updater);
                }
            }
            Message::OpenUninstallDialog => {
                if !self.downloading && !self.is_loading() && !updater::is_busy(&self.updater) {
                    self.uninstall_options = updater::UninstallOptions::default();
                    self.show_uninstall_dialog = true;
                }
            }
            Message::CloseUninstallDialog => self.show_uninstall_dialog = false,
            Message::SetKeepSettings(value) => self.uninstall_options.keep_settings = value,
            Message::SetKeepTools(value) => self.uninstall_options.keep_tools = value,
            Message::SetKeepCache(value) => self.uninstall_options.keep_cache = value,
            Message::SetKeepLogs(value) => self.uninstall_options.keep_logs = value,
            Message::SetKeepOtherData(value) => self.uninstall_options.keep_other_data = value,
            Message::ConfirmUninstall => {
                if self.downloading || self.is_loading() || updater::is_busy(&self.updater) {
                    self.push_log(UiLogLevel::Warn, "Finish or stop active downloads and installation tasks before uninstalling.");
                } else {
                    self.show_uninstall_dialog = false;
                    if self.uninstall_options.keep_settings {
                        self.save_config();
                    }
                    updater::spawn_uninstall_app(
                        &self.updater,
                        self.uninstall_options.clone(),
                        PathBuf::from(self.output_dir.clone()),
                    );
                }
            }
            Message::DismissUpdatePrompt => self.update_prompt = false,
            Message::UpdatePromptView => {
                self.update_prompt = false;
                if self.current_view == View::Settings {
                    self.save_config();
                }
                self.current_view = View::Installation;
            }
            Message::Noop => {},
            Message::PollWorker => {self.tick = self.tick.wrapping_add(1);
                self.updater_seen = updater::generation(&self.updater);
                if updater::take_exit_request(&self.updater) {
                    // An uninstall already saved the settings it keeps. Saving again
                    // here would recreate the config folder it is about to delete.
                    if !updater::is_uninstalling(&self.updater) {
                        self.save_config();
                    }
                    return iced::exit();
                }
                if let Some(error) = updater::take_uninstall_error(&self.updater) {
                    self.push_log(UiLogLevel::Error, format!("Uninstallation could not be started: {error}"));
                    self.log_attention = 16;
                    self.needs_log_window = true;
                }
                if updater::take_app_prompt(&self.updater) {
                    self.update_prompt = true;
                }
                self.log_attention = self.log_attention.saturating_sub(1);
                let events = {
                    let mut g = self.inbox.lock().unwrap_or_else(|e| e.into_inner());
                    std::mem::take(&mut *g)
                };
                for e in events {
                    self.handle_worker_event(e);
                }
                if self.needs_log_window {
                    self.needs_log_window = false;
                    return self.open_log_window();
                }
            }
        }
        Task::none()
    }

    pub(super) fn push_log(&mut self, level: UiLogLevel, text: impl Into<String>) {
        self.logs.push_back(crate::ui::LogLine {
            level,
            text: text.into(),
        });
        while self.logs.len() > 400 {
            self.logs.pop_front();
        }
    }

    fn warn_visibly(&mut self, text: &str) {
        self.push_log(UiLogLevel::Warn, text);
        self.log_attention = 16;
        self.needs_log_window = true;
    }

    fn open_output_dir(&self) {
        let path = std::path::Path::new(&self.output_dir);
        let _ = std::fs::create_dir_all(path);
        #[cfg(target_os = "windows")]
        let _ = std::process::Command::new("explorer").arg(path).spawn();
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(path).spawn();
        #[cfg(target_os = "linux")]
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }

    fn add_url(&mut self) {
        if self.downloading {
            return;
        }
        let url = normalize_url(&self.url_input);
        if url.is_empty() {
            return;
        }

        let source = detect_source(&url);
        if source == Source::Unsupported {
            self.warn_visibly("Only SoundCloud, YouTube, Spotify and Deezer links are supported.");
            return;
        }
        if source == Source::Spotify && !spotify::is_spotify_url(&url) {
            self.warn_visibly("Only Spotify track, album and playlist links are supported.");
            return;
        }
        if source == Source::ITunes && !itunes::is_itunes_url(&url) {
            self.warn_visibly("Only Apple Music track and album links are supported.");
            return;
        }
        if source == Source::Deezer && !deezer::is_deezer_url(&url) {
            self.warn_visibly(
                "Only Deezer track, album and playlist links are supported. \
                 Short links (link.deezer.com) are not supported — paste the full deezer.com URL.",
            );
            return;
        }

        let already_queued = self
            .queue
            .iter()
            .any(|item| item.url == url || item.tracks().any(|t| t.url == url));
        if already_queued {
            self.warn_visibly(&format!("This URL is already in the queue: {url}"));
            return;
        }

        let id = self.next_id;
        self.next_id += 1;
        let is_playlist = match source {
            Source::Spotify => spotify::is_spotify_playlist_url(&url),
            Source::Deezer => deezer::is_deezer_playlist_url(&url),
            Source::ITunes => itunes::is_itunes_playlist_url(&url),
            _ => is_playlist_url(&url, source),
        };
        let display_name = if is_playlist {
            READING_PLAYLIST.to_string()
        } else {
            guess_name_from_url(&url, source)
        };
        let mut item = QueueItem::new(id, url.clone(), source, display_name);

        let cookies = self.cookies_path();
        let inbox = self.inbox.clone();
        let cancel = self.resolve_cancel.clone();
        match source {
            Source::Spotify => {
                item.resolving = true;
                if is_playlist {
                    spotify::spawn_resolve_playlist(id, url, inbox, cancel);
                } else {
                    spotify::spawn_resolve_track(id, url, inbox, cancel);
                }
            }
            Source::Deezer => {
                item.resolving = true;
                if is_playlist {
                    deezer::spawn_resolve_playlist(id, url, inbox, cancel);
                } else {
                    deezer::spawn_resolve_track(id, url, inbox, cancel);
                }
            }
            Source::ITunes => {
                item.resolving = true;
                if is_playlist {
                    itunes::spawn_resolve_playlist(id, url, inbox, cancel);
                } else {
                    itunes::spawn_resolve_track(id, url, inbox, cancel);
                }
            }
            _ if is_playlist => {
                item.resolving = true;
                spawn_resolve(id, url, cookies, inbox, cancel);
            }
            _ => spawn_preview(id, url, cookies, inbox, cancel),
        }

        self.queue.push(item);
        self.url_input.clear();
    }

    fn remove_track(&mut self, id: u64) {
        if self.downloading {
            return;
        }
        // Removing a playlist header removes the whole playlist; removing a child
        // only removes that one track.
        self.queue.retain(|i| i.id != id);
        for item in &mut self.queue {
            if let Some(group) = &mut item.group {
                group.children.retain(|c| c.id != id);
            }
        }
        // A header with no children left has nothing to show; drop it.
        self.queue
            .retain(|i| i.group.as_ref().is_none_or(|g| !g.children.is_empty()));
    }

    fn start_queue(&mut self) {
        if self.downloading || self.queue.is_empty() {
            return;
        }

        let output_dir = PathBuf::from(self.output_dir.trim());
        if output_dir.as_os_str().is_empty() {
            self.push_log(UiLogLevel::Error, "Choose an output folder first.");
            self.current_view = View::Settings;
            return;
        }
        if let Err(e) = std::fs::create_dir_all(&output_dir) {
            self.push_log(UiLogLevel::Error, format!("Cannot create output folder: {e}"));
            return;
        }
        let cookies = match self.cookies_path() {
            None => None,
            Some(p) if p.exists() => Some(p),
            Some(p) => {
                self.push_log(
                    UiLogLevel::Error,
                    format!("Cookies file not found: {}", p.display()),
                );
                return;
            }
        };
        if self.is_loading() {
            self.push_log(UiLogLevel::Warn, "Wait for everything to finish loading first.");
            return;
        }

        let mut items = Vec::new();
        let mut used_folders = std::collections::HashSet::new();
        for entry in &self.queue {
            let folder = if self.group_playlists {
                entry.playlist_title.as_ref().map(|title| {
                    let base = crate::backend::sanitize_filename(title);
                    let mut candidate = base.clone();
                    let mut counter = 1;
                    // Check both the current batch AND the filesystem to ensure
                    // no existing folder is ever mixed with a new playlist.
                    while used_folders.contains(&candidate) || output_dir.join(&candidate).is_dir() {
                        candidate = format!("{} ({})", base, counter);
                        counter += 1;
                    }
                    used_folders.insert(candidate.clone());
                    candidate
                })
            } else {
                None
            };
            for t in entry.tracks().filter(|t| !t.unmatched()) {
                let download_url = t.resolved_url.clone().unwrap_or_else(|| t.url.clone());
                items.push((
                    t.id,
                    download_url,
                    t.spotify.clone(),
                    t.deezer.clone(),
                    t.itunes.clone(),
                    folder.clone(),
                ));
            }
        }
        if items.is_empty() {
            self.push_log(
                UiLogLevel::Warn,
                "Nothing to download: no track has matching audio.",
            );
            return;
        }

        for item in &mut self.queue {
            item.for_each_track_mut(|t| {
                if !t.unmatched() {
                    t.status = ItemStatus::Pending;
                    t.progress = 0.0;
                }
            });
        }
        self.downloading = true;
        self.stop_flag.store(false, Ordering::SeqCst);
        self.logs.clear();
        self.push_log(UiLogLevel::Info, "Queue started.");
        self.status = "Starting\u{2026}".to_string();
        spawn_worker(
            items,
            output_dir,
            cookies,
            self.inbox.clone(),
            self.stop_flag.clone(),
            self.skip_existing,
        );
    }

    fn stop_all(&mut self) {
        let was_resolving = self.queue.iter().any(|i| i.resolving);

        self.stop_flag.store(true, Ordering::SeqCst);
        self.resolve_cancel.store(true, Ordering::SeqCst);
        self.resolve_cancel = Arc::new(AtomicBool::new(false));

        for item in &mut self.queue {
            if item.resolving {
                item.resolving = false;
                item.status = ItemStatus::Stopped;
                if item.display_name == READING_PLAYLIST {
                    item.display_name = "Playlist".to_string();
                }
            }
            item.for_each_track_mut(|t| {
                if matches!(t.thumbnail, ThumbState::Loading) {
                    t.thumbnail = ThumbState::Failed;
                }
                if t.display_name == FETCHING_TITLE {
                    t.display_name = fallback_name(&t.url, t.source);
                }
            });
        }

        if self.downloading {
            // The scheduler drops queued jobs without reporting them, so mark
            // them here; running jobs report Stopped themselves.
            for item in &mut self.queue {
                item.for_each_track_mut(|t| {
                    if t.status == ItemStatus::Pending && !t.unmatched() {
                        t.status = ItemStatus::Stopped;
                    }
                });
            }
            self.status = "Stopping\u{2026}".to_string();
            self.push_log(
                UiLogLevel::Warn,
                "Stop requested - terminating active downloads.",
            );
        } else if was_resolving {
            self.push_log(UiLogLevel::Warn, "Stopped loading playlists and tracks.");
        }
    }
}