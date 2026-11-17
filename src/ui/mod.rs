pub mod icons;
pub mod progress;
pub mod queue;

use crate::backend::Source;
use iced::widget::image;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View { Downloads, Settings, Installation }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus { Pending, Downloading, Done, Failed, Unavailable, Skipped, Stopped }

impl ItemStatus {
    pub fn label(self) -> &'static str {
        match self {
            ItemStatus::Pending => "Pending",
            ItemStatus::Downloading => "Downloading",
            ItemStatus::Done => "Done",
            ItemStatus::Failed => "Failed",
            ItemStatus::Unavailable => "Unavailable",
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
    pub deezer: Option<crate::backend::deezer::DeezerMeta>,
    pub itunes: Option<crate::backend::itunes::ITunesMeta>,
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
            deezer: None,
            itunes: None,
            playlist_title: None,
        }
    }

    /// True when this row came from a metadata-only service (Spotify, Deezer,
    /// Apple Music) whose audio has to be matched on YouTube/SoundCloud first.
    pub fn uses_metadata_source(&self) -> bool {
        self.spotify.is_some()
            || self.deezer.is_some()
            || self.itunes.is_some()
            || matches!(self.source, Source::Spotify | Source::Deezer | Source::ITunes)
    }

    /// Metadata is known but the audio match is still in flight.
    pub fn awaiting_match(&self) -> bool {
        self.uses_metadata_source()
            && self.resolved_url.is_none()
            && self.status == ItemStatus::Pending
    }

    /// A metadata-source row with no downloadable audio. It cannot be sent to
    /// the downloader, and it is not a download failure.
    pub fn unmatched(&self) -> bool {
        self.uses_metadata_source() && self.resolved_url.is_none()
    }

    /// The row is still loading something (metadata, audio match or cover)
    /// and must not be downloaded yet.
    pub fn is_loading(&self) -> bool {
        self.resolving
            || self.awaiting_match()
            || (matches!(self.thumbnail, ThumbState::Loading) && self.status == ItemStatus::Pending)
    }

    /// A genuine download-stage problem worth retrying: failed, unavailable or
    /// stopped. Metadata and matching failures do not count.
    pub fn needs_retry(&self) -> bool {
        !self.unmatched()
            && matches!(
                self.status,
                ItemStatus::Failed | ItemStatus::Unavailable | ItemStatus::Stopped
            )
    }

    /// All downloadable tracks in this row. For a playlist header that's only
    /// the children; for a single track that's the row itself.
    pub fn tracks(&self) -> impl Iterator<Item = &QueueItem> {
        let is_header = self.group.is_some();
        std::iter::once(self)
            .filter(move |_| !is_header)
            .chain(self.group.iter().flat_map(|g| g.children.iter()))
    }

    pub fn for_each_track_mut(&mut self, mut f: impl FnMut(&mut QueueItem)) {
        if let Some(group) = &mut self.group {
            for child in &mut group.children {
                f(child);
            }
        } else {
            f(self);
        }
    }

    pub fn find_mut(&mut self, id: u64) -> Option<&mut QueueItem> {
        if self.id == id {
            return Some(self);
        }
        self.group.as_mut()?.children.iter_mut().find(|c| c.id == id)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum UiLogLevel { Info, Warn, Error }

#[derive(Debug, Clone)]
pub struct LogLine { pub level: UiLogLevel, pub text: String }

pub use self::progress::custom_progress_bar;