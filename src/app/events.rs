//! Applies events posted by the background workers to the queue.

use super::helpers::{guess_name_from_url, FETCHING_TITLE};
use super::App;
use crate::backend::{
    detect_source, spawn_preview, spawn_previews, spawn_thumbnails, LogLevel, PlaylistEntry,
    Source, Thumb, TrackResult, WorkerEvent,
};
use crate::ui::{ItemStatus, PlaylistGroup, QueueItem, ThumbState, UiLogLevel};

fn thumb_state(thumb: Option<Thumb>) -> ThumbState {
    match thumb {
        Some((w, h, rgba)) => ThumbState::Ready(iced::widget::image::Handle::from_rgba(w, h, rgba)),
        None => ThumbState::Failed,
    }
}

fn usable_track_title(title: &str) -> bool {
    let title = title.trim();
    !title.is_empty()
        && !matches!(
            title.to_ascii_lowercase().as_str(),
            "untitled" | "unknown" | "unknown video" | "unknown track" | "track"
        )
}

fn is_artist_avatar_thumb(url: &str) -> bool {
    let url = url.to_ascii_lowercase();
    url.contains("avatars-") || url.contains("/avatars/")
}

impl App {
    pub(super) fn handle_worker_event(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::TrackStarted { id } => {
                if let Some(i) = self.find_item_mut(id) {
                    i.status = ItemStatus::Downloading;
                    i.progress = 0.0;
                }
                self.refresh_download_status();
            }
            WorkerEvent::TrackTitle { id, title } => {
                if let Some(i) = self.find_item_mut(id) { i.display_name = title; }
            }
            WorkerEvent::Progress { id, value } => {
                if let Some(i) = self.find_item_mut(id) { i.progress = value; }
            }
            WorkerEvent::Thumbnail { id, thumb } => {
                if let Some(i) = self.find_item_mut(id) { i.thumbnail = thumb_state(thumb); }
            }
            WorkerEvent::Log { level, text } => {
                let lvl = match level {
                    LogLevel::Info => UiLogLevel::Info,
                    LogLevel::Warn => UiLogLevel::Warn,
                    LogLevel::Error => UiLogLevel::Error,
                };
                self.push_log(lvl, text);
            }
            WorkerEvent::TrackFinished { id, result } => {
                if let Some(i) = self.find_item_mut(id) {
                    i.status = match result {
                        TrackResult::Ok => { i.progress = 1.0; ItemStatus::Done }
                        TrackResult::Drm => ItemStatus::DrmSkipped,
                        TrackResult::Unavailable => ItemStatus::Unavailable,
                        TrackResult::Error => ItemStatus::Failed,
                        TrackResult::Stopped => ItemStatus::Stopped,
                    };
                }
                self.refresh_download_status();
            }
            WorkerEvent::QueueFinished { succeeded, failed, unavailable, drm, stopped } => {
                self.downloading = false;
                let verb = if stopped { "Stopped" } else { "Done" };
                self.status = format!(
                    "{verb} \u{2014} {succeeded} succeeded, {failed} failed, {unavailable} unavailable, {drm} DRM-skipped"
                );
                self.push_log(UiLogLevel::Info, self.status.clone());
                self.update_disk_space();
            }
            WorkerEvent::PreviewReady { id, title, thumb } => {
                if let Some(i) = self.find_item_mut(id) {
                    if usable_track_title(&title) {
                        i.display_name = title.trim().to_string();
                    } else if i.display_name == FETCHING_TITLE {
                        // Do not let an incomplete info JSON replace a useful track
                        // title with the literal placeholder "Untitled".
                        let fallback = guess_name_from_url(&i.url, i.source);
                        i.display_name = if fallback != FETCHING_TITLE && !fallback.trim().is_empty() {
                            fallback
                        } else if i.source == Source::YouTube {
                            "Unknown video".to_string()
                        } else {
                            "Unknown track".to_string()
                        };
                    }

                    if let Some(thumb) = thumb {
                        i.thumbnail = thumb_state(Some(thumb));
                    } else if matches!(&i.thumbnail, ThumbState::Loading) {
                        i.thumbnail = ThumbState::Failed;
                    }
                }
            }
            WorkerEvent::PreviewFailed { id, error } => {
                if let Some(i) = self.find_item_mut(id) {
                    i.thumbnail = ThumbState::Failed;
                    if i.display_name == FETCHING_TITLE {
                        let fallback = guess_name_from_url(&i.url, i.source);
                        i.display_name = if fallback != FETCHING_TITLE && !fallback.trim().is_empty() {
                            fallback
                        } else if i.source == Source::YouTube {
                            "Unknown video".to_string()
                        } else {
                            "Unknown track".to_string()
                        };
                    }
                }
                self.push_log(UiLogLevel::Warn, format!("Preview failed: {error}"));
            }
            WorkerEvent::PlaylistResolved { id, title, entries, main } => {
                self.apply_playlist(id, &title, entries, main);
            }
            WorkerEvent::PlaylistFailed { id, error } => {
                self.push_log(
                    UiLogLevel::Warn,
                    format!("Could not read playlist ({error}); adding it as a single track."),
                );
                let cookies = self.cookies_path();
                let inbox = self.inbox.clone();
                if let Some(i) = self.find_item_mut(id) {
                    i.resolving = false;
                    spawn_preview(id, i.url.clone(), cookies, inbox);
                }
            }
        }
    }

    /// Turns a resolved playlist into its main row plus a dropdown of the remaining tracks.
    /// The row keeps the id it was added with; every other track gets a fresh id.
    fn apply_playlist(&mut self, id: u64, title: &str, mut entries: Vec<PlaylistEntry>, main: usize) {
        if entries.is_empty() || !self.queue.iter().any(|q| q.id == id) {
            return; // the user removed the row while it was loading
        }

        let playlist_source = self.find_item_mut(id).map(|item| item.source);
        let is_soundcloud_playlist = playlist_source == Some(Source::SoundCloud);
        let main_entry = entries.remove(main.min(entries.len() - 1));

        // SoundCloud's --flat-playlist result is useful for expansion, but its
        // thumbnail can be the creator avatar and its track title may be missing.
        // Resolve the actual track URL for the main row instead of displaying the
        // playlist container's title/artwork or trusting incomplete flat metadata.
        let mut preview_jobs = Vec::new();
        let mut thumb_jobs = Vec::with_capacity(entries.len());
        if is_soundcloud_playlist {
            preview_jobs.push((id, main_entry.url.clone()));
        } else if let Some(url) = &main_entry.thumb_url {
            thumb_jobs.push((id, url.clone()));
        }

        let main_title = if is_soundcloud_playlist {
            // The first flat entry can carry playlist/creator metadata. Show a
            // neutral loading label until the individual track preview returns.
            FETCHING_TITLE.to_string()
        } else {
            main_entry.title.clone()
        };

        let mut children = Vec::with_capacity(entries.len());
        for entry in entries {
            let child_id = self.next_id;
            self.next_id += 1;
            let source = detect_source(&entry.url);
            let needs_track_preview = source == Source::SoundCloud
                && (!usable_track_title(&entry.title)
                    || entry.thumb_url.as_deref().map_or(true, is_artist_avatar_thumb));

            let child_title = if needs_track_preview && !usable_track_title(&entry.title) {
                FETCHING_TITLE.to_string()
            } else {
                entry.title.clone()
            };
            let mut child = QueueItem::new(
                child_id,
                entry.url.clone(),
                source,
                child_title,
            );

            if needs_track_preview {
                child.thumbnail = ThumbState::Loading;
                preview_jobs.push((child_id, entry.url.clone()));
            } else {
                match entry.thumb_url {
                    Some(url) => thumb_jobs.push((child_id, url)),
                    None => child.thumbnail = ThumbState::Failed,
                }
            }
            children.push(child);
        }
        let total = children.len() + 1;

        if let Some(item) = self.find_item_mut(id) {
            item.source = detect_source(&main_entry.url);
            item.url = main_entry.url;
            item.display_name = main_title;
            item.resolving = false;
            item.thumbnail = if is_soundcloud_playlist || main_entry.thumb_url.is_some() {
                ThumbState::Loading
            } else {
                ThumbState::Failed
            };
            if !children.is_empty() {
                item.group = Some(PlaylistGroup { children, expanded: false });
            }
        }

        self.push_log(UiLogLevel::Info, format!("Playlist \"{title}\": {total} track(s) added."));
        if !preview_jobs.is_empty() {
            spawn_previews(preview_jobs, self.cookies_path(), self.inbox.clone());
        }
        spawn_thumbnails(thumb_jobs, self.inbox.clone());
    }

    fn refresh_download_status(&mut self) {
        let (mut total, mut finished, mut active) = (0usize, 0usize, 0usize);
        for t in self.tracks() {
            total += 1;
            match t.status {
                ItemStatus::Downloading => active += 1,
                ItemStatus::Pending => {}
                _ => finished += 1,
            }
        }
        self.status = format!("Downloading {finished}/{total} ({active} active)\u{2026}");
    }
}
