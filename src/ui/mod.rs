pub mod icons;
pub mod progress;
pub mod queue;

use crate::backend::Source;
use iced::widget::image;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Downloads,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    Pending,
    Downloading,
    Done,
    Failed,
    Unavailable,
    DrmSkipped,
    Skipped,
    Stopped,
}

impl ItemStatus {
    pub fn label(self) -> &'static str {
        match self {
            ItemStatus::Pending => "Pending",
            ItemStatus::Downloading => "Downloading",
            ItemStatus::Done => "Done",
            ItemStatus::Failed => "Failed",
            ItemStatus::Unavailable => "Unavailable",
            ItemStatus::DrmSkipped => "DRM skipped",
            ItemStatus::Skipped => "Skipped",
            ItemStatus::Stopped => "Stopped",
        }
    }
}

#[derive(Clone)]
pub enum ThumbState {
    Loading,
    Ready(image::Handle),
    Failed,
}

#[derive(Clone)]
pub struct PlaylistGroup {
    pub children: Vec<QueueItem>,
    pub expanded: bool,
}

#[derive(Clone)]
pub struct QueueItem {
    pub id: u64,
    pub url: String,
    pub source: Source,
    pub status: ItemStatus,
    pub progress: f32,
    pub display_name: String,
    pub thumbnail: ThumbState,
    pub resolving: bool,
    pub group: Option<PlaylistGroup>,
    pub resolved_url: Option<String>,
    pub spotify: Option<crate::backend::spotify::SpotifyMeta>,
    pub playlist_title: Option<String>,
}

impl QueueItem {
    pub fn new(id: u64, url: String, source: Source, display_name: String) -> Self {
        Self {
            id,
            url,
            source,
            status: ItemStatus::Pending,
            progress: 0.0,
            display_name,
            thumbnail: ThumbState::Loading,
            resolving: false,
            group: None,
            resolved_url: None,
            spotify: None,
            playlist_title: None,
        }
    }
    pub fn tracks(&self) -> impl Iterator<Item = &QueueItem> {
        std::iter::once(self).chain(self.group.iter().flat_map(|g| g.children.iter()))
    }
    pub fn for_each_track_mut(&mut self, mut f: impl FnMut(&mut QueueItem)) {
        f(self);
        if let Some(group) = &mut self.group {
            group.children.iter_mut().for_each(f);
        }
    }
    pub fn find_mut(&mut self, id: u64) -> Option<&mut QueueItem> {
        if self.id == id {
            return Some(self);
        }
        self.group
            .as_mut()?
            .children
            .iter_mut()
            .find(|c| c.id == id)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum UiLogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub level: UiLogLevel,
    pub text: String,
}

pub use self::progress::custom_progress_bar;
