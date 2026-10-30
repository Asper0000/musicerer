//! YouTube-specific helpers: playlist detection and thumbnail URLs,
//! preview and download argument construction, and search term formatting.

pub mod download;
pub mod playlist;
pub mod preview;
pub mod search;

/// Title decorations YouTube uploaders commonly append in parentheses or
/// brackets, stripped from downloaded metadata before it reaches the tags.
/// Applied only to YouTube sources; other backends leave titles alone.
pub(crate) const UNWANTED_TITLE_SUFFIXES: &[&str] = &[
    r"\(Official Music Video\)",
    r"\(Official Video\)",
    r"\(Official Audio\)",
    r"\(Lyric Video\)",
    r"\(Audio\)",
    r"\(Visualizer\)",
    r"\[HD\]",
];