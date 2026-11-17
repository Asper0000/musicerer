//! Applies events posted by the background workers to the queue.

use super::helpers::{fallback_name, FETCHING_TITLE};
use super::App;
use crate::backend::spotify::PUBLIC_PAGE_LIMIT;
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
    fn update_parent_thumbnail(&mut self, child_id: u64, thumb: Option<Thumb>) {
        let Some(t) = thumb else { return; };
        let ready = thumb_state(Some(t));
        for parent in &mut self.queue {
            if let Some(group) = &mut parent.group {
                if group.children.iter().any(|c| c.id == child_id) {
                    if matches!(parent.thumbnail, ThumbState::Failed | ThumbState::Loading) {
                        parent.thumbnail = ready.clone();
                    }
                    break;
                }
            }
        }
    }

    /// A playlist header waiting on a child cover gives up once no child is
    /// still loading one, so it never spins forever.
    fn settle_header_thumbs(&mut self) {
        for parent in &mut self.queue {
            let Some(group) = &parent.group else { continue; };
            if !matches!(parent.thumbnail, ThumbState::Loading) {
                continue;
            }
            if group.children.iter().any(|c| matches!(c.thumbnail, ThumbState::Loading)) {
                continue;
            }
            parent.thumbnail = group
                .children
                .iter()
                .find(|c| matches!(c.thumbnail, ThumbState::Ready(_)))
                .map_or(ThumbState::Failed, |c| c.thumbnail.clone());
        }
    }

    pub(super) fn handle_worker_event(&mut self, event: WorkerEvent) {
        self.handle_event_inner(event);
        self.settle_header_thumbs();
    }

    fn handle_event_inner(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::TrackStarted { id } => {
                if let Some(i) = self.find_item_mut(id) {
                    i.status = ItemStatus::Downloading;
                    i.progress = 0.0;
                }
                self.refresh_download_status();
            }
            WorkerEvent::TrackTitle { id, title } => {
                if let Some(i) = self.find_item_mut(id) {
                    if !i.uses_metadata_source() {
                        i.display_name = title;
                    }
                }
            }
            WorkerEvent::Progress { id, value } => {
                if let Some(i) = self.find_item_mut(id) {
                    i.progress = value;
                }
            }
            WorkerEvent::Thumbnail { id, thumb } => {
                if let Some(i) = self.find_item_mut(id) {
                    let has_cover = matches!(i.thumbnail, ThumbState::Ready(_));
                    // Metadata rows keep a cover they already have. A Deezer
                    // row is always followed by DeezerTrackResolved, which sets
                    // its final cover, so a failed early fetch must not flash
                    // the failure icon in the meantime.
                    let keep = i.uses_metadata_source()
                        && (has_cover
                            || (thumb.is_none() && i.deezer.is_some() && i.awaiting_match()));
                    if !keep {
                        i.thumbnail = thumb_state(thumb.clone());
                    }
                }
                self.update_parent_thumbnail(id, thumb);
            }
            WorkerEvent::Log { level, text } => {
                let level = match level {
                    LogLevel::Info => UiLogLevel::Info,
                    LogLevel::Warn => UiLogLevel::Warn,
                    LogLevel::Error => UiLogLevel::Error,
                };
                self.push_log(level, text);
            }
            WorkerEvent::TrackFinished { id, result } => {
                if let Some(i) = self.find_item_mut(id) {
                    i.status = match result {
                        TrackResult::Ok => {
                            i.progress = 1.0;
                            ItemStatus::Done
                        }
                        TrackResult::Unavailable => ItemStatus::Unavailable,
                        TrackResult::Error => ItemStatus::Failed,
                        TrackResult::Skipped => {
                            i.progress = 1.0;
                            ItemStatus::Skipped
                        }
                        TrackResult::Stopped => ItemStatus::Stopped,
                    };
                }
                self.refresh_download_status();
            }
            WorkerEvent::QueueFinished {
                succeeded,
                failed,
                unavailable,
                skipped,
                stopped,
            } => {
                self.downloading = false;
                let verb = if stopped { "Stopped" } else { "Done" };
                self.status = format!(
                    "{verb} \u{2014} {succeeded} succeeded, {failed} failed, {unavailable} unavailable, {skipped} skipped"
                );
                self.push_log(UiLogLevel::Info, self.status.clone());
                self.update_disk_space();
                // Every track errored: surface the log so the cause is visible.
                if !stopped && succeeded == 0 && skipped == 0 && failed + unavailable > 0 {
                    self.log_attention = 16;
                    self.needs_log_window = true;
                }
            }
            WorkerEvent::PreviewReady { id, title, thumb, sc_meta } => {
                self.update_parent_thumbnail(id, thumb.clone());
                let Some(i) = self.find_item_mut(id) else {
                    return;
                };
                if i.uses_metadata_source() {
                    return;
                }
                // SoundCloud hydration carries publisher_metadata, so the
                // real artist and title can be shown before download. Fall
                // back to the yt-dlp title, then to the URL slug.
                let hydrated = sc_meta.as_ref().and_then(|sc| {
                    let title = sc.title.as_deref().map(str::trim).filter(|s| !s.is_empty());
                    let artist = sc.artist.as_deref().map(str::trim).filter(|s| !s.is_empty());
                    match (artist, title) {
                        (Some(a), Some(t)) => Some(format!("{a} - {t}")),
                        (None, Some(t)) => Some(t.to_string()),
                        _ => None,
                    }
                });
                if let Some(name) = hydrated {
                    i.display_name = name;
                } else if usable_track_title(&title) {
                    i.display_name = title.trim().to_string();
                } else if i.display_name == FETCHING_TITLE {
                    i.display_name = fallback_name(&i.url, i.source);
                }
                match thumb {
                    Some(thumb) => i.thumbnail = thumb_state(Some(thumb)),
                    None if matches!(i.thumbnail, ThumbState::Loading) => {
                        i.thumbnail = ThumbState::Failed
                    }
                    None => {}
                }
            }
            WorkerEvent::PreviewFailed { id, error } => {
                if let Some(i) = self.find_item_mut(id) {
                    i.thumbnail = ThumbState::Failed;
                    if i.display_name == FETCHING_TITLE {
                        i.display_name = fallback_name(&i.url, i.source);
                    }
                }
                self.push_log(UiLogLevel::Warn, format!("Preview failed: {error}"));
            }
            WorkerEvent::PlaylistResolved {
                id,
                title,
                entries,
                main,
            } => {
                self.apply_playlist(id, &title, entries, main);
            }
            WorkerEvent::PlaylistFailed { id, error } => {
                let cookies = self.cookies_path();
                let inbox = self.inbox.clone();
                let cancel = self.resolve_cancel.clone();
                let Some(i) = self.find_item_mut(id) else {
                    return;
                };
                if !i.resolving {
                    return;
                }
                i.resolving = false;
                spawn_preview(id, i.url.clone(), cookies, inbox, cancel);
                self.push_log(
                    UiLogLevel::Warn,
                    format!("Could not read playlist ({error}); adding it as a single track."),
                );
            }
            WorkerEvent::SpotifyResolved {
                id,
                meta,
                resolved_url,
                thumb,
            } => {
                let Some(i) = self.find_item_mut(id) else {
                    return;
                };
                if !i.resolving {
                    return;
                }
                i.resolving = false;
                i.display_name = meta.display_title();
                i.resolved_url = resolved_url;
                i.spotify = Some(meta);
                if let Some(thumb) = thumb {
                    i.thumbnail = thumb_state(Some(thumb));
                } else if matches!(i.thumbnail, ThumbState::Loading) {
                    i.thumbnail = ThumbState::Failed;
                }
                if i.resolved_url.is_none() {
                    i.status = ItemStatus::Failed;
                }
            }
            WorkerEvent::SpotifyFailed { id, title, error } => {
                let Some(i) = self.find_item_mut(id) else {
                    return;
                };
                if !i.resolving {
                    return;
                }
                i.resolving = false;
                i.status = ItemStatus::Failed;
                i.thumbnail = ThumbState::Failed;
                if !title.is_empty() {
                    i.display_name = title.clone();
                }
                self.push_log(
                    UiLogLevel::Error,
                    format!("Spotify resolve failed for \"{title}\": {error}"),
                );
                self.log_attention = 16;
                self.needs_log_window = true;
            }
            WorkerEvent::SpotifyTrackMatched {
                parent_id,
                entry_index,
                url,
            } => {
                let Some(item) = self.find_item_mut(parent_id) else {
                    return;
                };
                let target = item
                    .group
                    .as_mut()
                    .and_then(|g| g.children.get_mut(entry_index));
                let Some(child) = target else {
                    return;
                };
                if child.status == ItemStatus::Stopped {
                    return;
                }
                match url {
                    Some(u) => {
                        child.resolved_url = Some(u);
                    }
                    None => {
                        child.resolved_url = None;
                        child.status = ItemStatus::Failed;
                    }
                }
            }
            WorkerEvent::SpotifyPlaylistComplete { parent_id } => {
                if let Some(i) = self.find_item_mut(parent_id) {
                    i.resolving = false;
                }
                self.refresh_download_status();
            }
            WorkerEvent::DeezerResolved {
                id,
                meta,
                resolved_url,
                thumb,
            } => {
                let Some(i) = self.find_item_mut(id) else {
                    return;
                };
                if !i.resolving {
                    return;
                }
                i.resolving = false;
                i.display_name = meta.display_title();
                i.resolved_url = resolved_url;
                i.deezer = Some(meta);
                if let Some(thumb) = thumb {
                    i.thumbnail = thumb_state(Some(thumb));
                } else if matches!(i.thumbnail, ThumbState::Loading) {
                    i.thumbnail = ThumbState::Failed;
                }
                if i.resolved_url.is_none() {
                    i.status = ItemStatus::Failed;
                }
            }
            WorkerEvent::DeezerFailed { id, title, error } => {
                let Some(i) = self.find_item_mut(id) else {
                    return;
                };
                if !i.resolving {
                    return;
                }
                i.resolving = false;
                i.status = ItemStatus::Failed;
                i.thumbnail = ThumbState::Failed;
                if !title.is_empty() {
                    i.display_name = title.clone();
                }
                self.push_log(
                    UiLogLevel::Error,
                    format!("Deezer resolve failed for \"{title}\": {error}"),
                );
                self.log_attention = 16;
                self.needs_log_window = true;
            }
            WorkerEvent::DeezerTrackResolved {
                parent_id,
                entry_index,
                meta,
                resolved_url,
                thumb,
            } => {
                let Some(item) = self.find_item_mut(parent_id) else {
                    return;
                };
                let target = item
                    .group
                    .as_mut()
                    .and_then(|g| g.children.get_mut(entry_index));
                let Some(child) = target else {
                    return;
                };
                if child.status == ItemStatus::Stopped {
                    return;
                }
                child.display_name = meta.display_title();
                child.deezer = Some(meta);
                match resolved_url {
                    Some(u) => {
                        child.resolved_url = Some(u);
                    }
                    None => {
                        child.resolved_url = None;
                        child.status = ItemStatus::Failed;
                    }
                }
                if let Some(t) = thumb {
                    child.thumbnail = thumb_state(Some(t));
                } else if matches!(child.thumbnail, ThumbState::Loading) {
                    child.thumbnail = ThumbState::Failed;
                }
            }
            WorkerEvent::ITunesResolved { id, meta, resolved_url, thumb } => {
                let Some(i) = self.find_item_mut(id) else { return; };
                if !i.resolving { return; }
                i.resolving = false;
                i.display_name = meta.display_title();
                i.resolved_url = resolved_url;
                i.itunes = Some(meta);
                if let Some(thumb) = thumb {
                    i.thumbnail = thumb_state(Some(thumb));
                } else if matches!(i.thumbnail, ThumbState::Loading) {
                    i.thumbnail = ThumbState::Failed;
                }
                if i.resolved_url.is_none() { i.status = ItemStatus::Failed; }
            }
            WorkerEvent::ITunesFailed { id, title, error } => {
                let Some(i) = self.find_item_mut(id) else { return; };
                if !i.resolving { return; }
                i.resolving = false;
                i.status = ItemStatus::Failed;
                i.thumbnail = ThumbState::Failed;
                if !title.is_empty() { i.display_name = title.clone(); }
                self.push_log(UiLogLevel::Error, format!("iTunes resolve failed for \"{title}\": {error}"));
                self.log_attention = 16;
                self.needs_log_window = true;
            }
            WorkerEvent::ITunesTrackMatched { parent_id, entry_index, url } => {
                let Some(item) = self.find_item_mut(parent_id) else { return; };
                let target = item.group.as_mut().and_then(|g| g.children.get_mut(entry_index));
                let Some(child) = target else { return; };
                if child.status == ItemStatus::Stopped { return; }
                match url {
                    Some(u) => { child.resolved_url = Some(u); }
                    None => {
                        child.resolved_url = None;
                        child.status = ItemStatus::Failed;
                    }
                }
            }
            WorkerEvent::ITunesPlaylistComplete { parent_id } => {
                if let Some(i) = self.find_item_mut(parent_id) { i.resolving = false; }
                self.refresh_download_status();
            }
            WorkerEvent::DeezerPlaylistComplete { parent_id } => {
                if let Some(i) = self.find_item_mut(parent_id) {
                    i.resolving = false;
                }
                self.refresh_download_status();
            }
        }
    }

    /// Turns a resolved playlist into a visual-only header plus a dropdown of every
    /// track. The header keeps the playlist name and cover; every entry becomes a
    /// freely removable child.
    fn apply_playlist(
        &mut self,
        id: u64,
        title: &str,
        entries: Vec<PlaylistEntry>,
        _main: usize,
    ) {
        let Some(playlist_source) = self
            .find_item_mut(id)
            .filter(|i| i.resolving)
            .map(|i| i.source)
        else {
            return;
        };
        if entries.is_empty() {
            return;
        }

        // SoundCloud entries often carry the artist avatar; skip it so the header
        // doesn't show it and then swap to a real cover.
        let header_thumb_url = entries
            .iter()
            .filter_map(|e| e.thumb_url.as_deref())
            .find(|u| !is_artist_avatar_thumb(u))
            .map(str::to_string);

        let mut preview_jobs = Vec::new();
        let mut thumb_jobs: Vec<(u64, String)> = Vec::with_capacity(entries.len() + 1);

        let mut children = Vec::with_capacity(entries.len());
        for entry in entries {
            let child_id = self.next_id;
            self.next_id += 1;

            let is_spotify = entry.spotify.is_some();
            let is_deezer = entry.deezer.is_some();
            let is_itunes = entry.itunes.is_some();
            let source = if is_spotify {
                Source::Spotify
            } else if is_deezer {
                Source::Deezer
            } else if is_itunes {
                Source::ITunes
            } else {
                detect_source(&entry.url)
            };

            let needs_track_preview = !is_spotify
                && !is_deezer
                && (!usable_track_title(&entry.title)
                    || entry.thumb_url.is_none()
                    || (source == Source::SoundCloud && entry.thumb_url.as_deref().is_none_or(is_artist_avatar_thumb)));

            let child_title = if needs_track_preview && !usable_track_title(&entry.title) {
                FETCHING_TITLE.to_string()
            } else if let Some(sp) = &entry.spotify {
                sp.display_title()
            } else if let Some(dz) = &entry.deezer {
                dz.display_title()
            } else if let Some(it) = &entry.itunes {
                it.display_title()
            } else {
                entry.title.clone()
            };
            let mut child = QueueItem::new(child_id, entry.url.clone(), source, child_title);

            if is_spotify || is_deezer || is_itunes {
                let prefix = if is_spotify { "spotify:" } else if is_deezer { "deezer:" } else { "itunes:" };
                if entry.url.starts_with(&format!("{prefix}pending")) {
                    // still resolving
                } else if entry.url.starts_with(prefix) {
                    child.status = ItemStatus::Failed;
                } else {
                    child.resolved_url = Some(entry.url.clone());
                }
            }

            child.spotify = entry.spotify.clone();
            child.deezer = entry.deezer.clone();
            child.itunes = entry.itunes.clone();

            if needs_track_preview {
                child.thumbnail = ThumbState::Loading;
                preview_jobs.push((child_id, entry.url.clone()));
            } else {
                match entry.thumb_url.clone() {
                    Some(url) => thumb_jobs.push((child_id, url)),
                    None => child.thumbnail = ThumbState::Failed,
                }
            }
            children.push(child);
        }
        let total = children.len();

        if let Some(item) = self.find_item_mut(id) {
            item.display_name = title.to_string();
            item.playlist_title = Some(title.to_string());
            item.resolving = matches!(playlist_source, Source::Spotify | Source::Deezer | Source::ITunes);
            item.thumbnail = if let Some(url) = &header_thumb_url {
                thumb_jobs.push((id, url.clone()));
                ThumbState::Loading
            } else {
                // Wait for a child's cover (see update_parent_thumbnail); the
                // header settles to Failed if none arrives (settle_header_thumbs).
                ThumbState::Loading
            };
            if !children.is_empty() {
                item.group = Some(PlaylistGroup {
                    children,
                    expanded: false,
                });
            }
        }

        self.push_log(
            UiLogLevel::Info,
            format!("Playlist \"{title}\": {total} track(s) added."),
        );
        if playlist_source == Source::Spotify && total >= PUBLIC_PAGE_LIMIT {
            self.push_log(
                UiLogLevel::Warn,
                format!("Spotify only exposes about {PUBLIC_PAGE_LIMIT} tracks of a public playlist; any further tracks are not included."),
            );
            self.status = format!("Spotify playlist limited to about {PUBLIC_PAGE_LIMIT} tracks.");
        }

        if !preview_jobs.is_empty() {
            spawn_previews(
                preview_jobs,
                self.cookies_path(),
                self.inbox.clone(),
                self.resolve_cancel.clone(),
            );
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