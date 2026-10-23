pub mod icons;
pub mod progress;
pub mod queue;

use crate::backend::Source;
use iced::widget::image;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View { Downloads, Settings }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus { Pending, Downloading, Done, Failed, DrmSkipped, Stopped }

impl ItemStatus {
    pub fn label(self) -> &'static str {
        match self {
            ItemStatus::Pending => "Pending",
            ItemStatus::Downloading => "Downloading",
            ItemStatus::Done => "Done",
            ItemStatus::Failed => "Failed",
            ItemStatus::DrmSkipped => "DRM skipped",
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
pub struct QueueItem {
    pub id: u64,
    pub url: String,
    pub source: Source,
    pub status: ItemStatus,
    pub progress: f32,
    pub display_name: String,
    pub thumbnail: ThumbState,
}

#[derive(Debug, Clone, Copy)]
pub enum UiLogLevel { Info, Warn, Error }

#[derive(Debug, Clone)]
pub struct LogLine { pub level: UiLogLevel, pub text: String }

pub use self::progress::custom_progress_bar;