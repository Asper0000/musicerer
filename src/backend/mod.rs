mod audio;
mod download;
mod playlist;
mod preview;
mod process;
mod scheduler;
mod tagging;
mod utils;
pub mod soundcloud_fallback;
pub mod vpn;

pub use download::spawn_worker;
pub use playlist::{is_playlist_url, spawn_resolve, spawn_thumbnails, PlaylistEntry};
pub use preview::{spawn_preview, spawn_previews};

use std::sync::{Arc, Mutex};

/// Shared mailbox the worker threads post events into and the UI drains.
pub type Inbox = Arc<Mutex<Vec<WorkerEvent>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackResult { Ok, Drm, Unavailable, Error, Stopped }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source { SoundCloud, YouTube, Unsupported }

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::SoundCloud => "SoundCloud",
            Source::YouTube => "YouTube",
            Source::Unsupported => "Unsupported",
        }
    }
}

pub fn normalize_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() { return String::new(); }
    if trimmed.contains("://") { trimmed.to_string() }
    else { format!("https://{trimmed}") }
}

pub fn detect_source(url: &str) -> Source {
    let normalized = normalize_url(url);
    let Ok(parsed) = url::Url::parse(&normalized) else { return Source::Unsupported; };
    let Some(host) = parsed.host_str() else { return Source::Unsupported; };
    let host = host.to_ascii_lowercase();
    if host == "youtu.be" || host == "youtube.com" || host.ends_with(".youtube.com") {
        Source::YouTube
    } else if host == "soundcloud.com" || host.ends_with(".soundcloud.com") {
        Source::SoundCloud
    } else {
        Source::Unsupported
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel { Info, Warn, Error }

/// A decoded, ready-to-display square thumbnail: `(width, height, rgba)`.
pub type Thumb = (u32, u32, Vec<u8>);

/// Events are keyed by the queue item's stable `id` (never by position), because
/// tracks finish out of order and playlist children shift positions around.
#[derive(Debug)]
pub enum WorkerEvent {
    TrackStarted { id: u64 },
    TrackTitle { id: u64, title: String },
    Progress { id: u64, value: f32 },
    Thumbnail { id: u64, thumb: Option<Thumb> },
    Log { level: LogLevel, text: String },
    TrackFinished { id: u64, result: TrackResult },
    QueueFinished { succeeded: usize, failed: usize, unavailable: usize, drm: usize, stopped: bool },

    PreviewReady { id: u64, title: String, thumb: Option<Thumb> },
    PreviewFailed { id: u64, error: String },

    PlaylistResolved { id: u64, title: String, entries: Vec<PlaylistEntry>, main: usize },
    PlaylistFailed { id: u64, error: String },
}

/// Progress and log spam may be dropped under backpressure; state changes never are,
/// otherwise a lost `TrackFinished` would leave the UI stuck in "downloading".
pub(crate) fn emit(inbox: &Inbox, event: WorkerEvent) {
    let droppable = matches!(event, WorkerEvent::Progress { .. } | WorkerEvent::Log { .. });
    let mut guard = inbox.lock().unwrap_or_else(|e| e.into_inner());
    if !droppable || guard.len() < 2_000 { guard.push(event); }
}

pub(crate) fn log(inbox: &Inbox, level: LogLevel, text: impl Into<String>) {
    emit(inbox, WorkerEvent::Log { level, text: text.into() });
}


#[cfg(test)]
mod tests {
    use super::{detect_source, normalize_url, Source};

    #[test]
    fn normalizes_input_without_rewriting_existing_schemes() {
        assert_eq!(normalize_url("  youtube.com/watch?v=abc  "), "https://youtube.com/watch?v=abc");
        assert_eq!(normalize_url("https://soundcloud.com/artist/track"), "https://soundcloud.com/artist/track");
        assert_eq!(normalize_url("   "), "");
    }

    #[test]
    fn detects_supported_hosts_without_matching_lookalikes() {
        assert_eq!(detect_source("youtube.com/watch?v=abc"), Source::YouTube);
        assert_eq!(detect_source("https://youtu.be/abc"), Source::YouTube);
        assert_eq!(detect_source("soundcloud.com/artist/track"), Source::SoundCloud);
        assert_eq!(detect_source("https://music.youtube.com/watch?v=abc"), Source::YouTube);
        assert_eq!(detect_source("https://youtube.com.example.org/watch?v=abc"), Source::Unsupported);
        assert_eq!(detect_source("https://example.com/soundcloud.com/artist/track"), Source::Unsupported);
    }
}
