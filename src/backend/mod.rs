mod audio;
mod download;
mod preview;
mod utils;
pub mod soundcloud_fallback;
pub mod vpn;

pub use download::spawn_worker;
pub use preview::spawn_preview;

use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackResult { Ok, Drm, Error, Stopped }

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

#[derive(Debug)]
pub enum WorkerEvent {
    TrackStarted { index: usize },
    TrackTitle { index: usize, title: String },
    Progress { index: usize, value: f32 },
    Thumbnail { index: usize, width: u32, height: u32, rgba: Vec<u8> },
    Log { level: LogLevel, text: String },
    TrackFinished { index: usize, result: TrackResult },
    QueueFinished { succeeded: usize, failed: usize, drm: usize, stopped: bool },

    PreviewReady { id: u64, title: String, thumb: Option<(u32, u32, Vec<u8>)> },
    PreviewFailed { id: u64, error: String },
}

pub(crate) fn emit(inbox: &Arc<Mutex<Vec<WorkerEvent>>>, event: WorkerEvent) {
    let mut guard = inbox.lock().unwrap_or_else(|e| e.into_inner());
    if guard.len() < 2_000 { guard.push(event); }
}

pub(crate) fn log(inbox: &Arc<Mutex<Vec<WorkerEvent>>>, level: LogLevel, text: impl Into<String>) {
    emit(inbox, WorkerEvent::Log { level, text: text.into() });
}
