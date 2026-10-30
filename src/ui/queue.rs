use crate::backend::Source;
use crate::theme;
use crate::ui::{icons::*, ItemStatus, QueueItem, ThumbState};
use iced::widget::{button, column, container, image, row, svg, text, Column, Space};
use iced::{Alignment, Color, Element, Length};

#[derive(Clone, Copy)]
enum RowKind {
    Top(usize),
    Child,
}

#[derive(Clone, Copy)]
struct Dropdown {
    expanded: bool,
    done: usize,
    total: usize,
}


fn is_completed(status: ItemStatus) -> bool {
    matches!(
        status,
        ItemStatus::Done
            | ItemStatus::Failed
            | ItemStatus::Unavailable
            | ItemStatus::DrmSkipped
            | ItemStatus::Skipped
            | ItemStatus::Stopped
    )
}


fn playlist_status(children: &[QueueItem], downloading: bool) -> ItemStatus {
    if children.is_empty() {
        return ItemStatus::Pending;
    }
    if children.iter().any(|c| matches!(c.status, ItemStatus::Failed | ItemStatus::Unavailable | ItemStatus::DrmSkipped)) {
        return ItemStatus::Failed;
    }
    if children.iter().all(|c| matches!(c.status, ItemStatus::Done | ItemStatus::Skipped)) {
        return ItemStatus::Done;
    }
    if downloading || children.iter().any(|c| c.status == ItemStatus::Downloading) {
        return ItemStatus::Downloading;
    }
    ItemStatus::Pending
}

pub fn queue_group<'a, M>(
    index: usize,
    item: &'a QueueItem,
    downloading: bool,
    on_remove: impl Fn(u64) -> M + Copy + 'a,
    on_toggle: impl Fn(u64) -> M + Copy + 'a,
    tick: u32,
) -> Element<'a, M>
where
    M: Clone + 'a,
{
    let group = item.group.as_ref().filter(|g| !g.children.is_empty());
    let dropdown = group.map(|g| Dropdown {
        expanded: g.expanded,
        done: g.children.iter().filter(|c| is_completed(c.status)).count(),
        total: g.children.len(),
    });

    let main = queue_row(
        RowKind::Top(index),
        item,
        downloading,
        dropdown,
        on_remove,
        on_toggle,
        tick,
    );

    let Some(group) = group.filter(|g| g.expanded) else {
        return main;
    };

    let children = Column::with_children(
        group
            .children
            .iter()
            .map(|child| {
                queue_row(RowKind::Child, child, downloading, None, on_remove, on_toggle, tick)
            })
            .collect::<Vec<_>>(),
    )
    .spacing(3);

    column![main, container(children).padding(0.0)]
        .spacing(3)
        .into()
}

fn queue_row<'a, M>(
    kind: RowKind,
    item: &'a QueueItem,
    downloading: bool,
    dropdown: Option<Dropdown>,
    on_remove: impl Fn(u64) -> M + 'a,
    on_toggle: impl Fn(u64) -> M + 'a,
    tick: u32,
) -> Element<'a, M>
where
    M: Clone + 'a,
{
    let is_header = item.group.is_some();
    let effective_status = if is_header {
        item.group.as_ref().map_or(ItemStatus::Pending, |g| playlist_status(&g.children, downloading))
    } else {
        item.status
    };

    let (status_color, status_icon) = if is_header {
        if item.resolving {
            (theme::text_muted(), ICON_STATUS_PENDING)
        } else {
            match effective_status {
                ItemStatus::Pending => (theme::text_muted(), ICON_STATUS_PENDING),
                ItemStatus::Downloading => (theme::primary(), ICON_STATUS_DOWNLOADING),
                ItemStatus::Done => (theme::success(), ICON_STATUS_CHECK),
                ItemStatus::Failed | ItemStatus::Unavailable | ItemStatus::DrmSkipped => (theme::danger(), ICON_STATUS_X),
                _ => (theme::text_muted(), ICON_STATUS_PENDING),
            }
        }
    } else {
        match item.status {
            ItemStatus::Pending => (theme::text_muted(), ICON_STATUS_PENDING),
            ItemStatus::Downloading => (theme::primary(), ICON_STATUS_DOWNLOADING),
            ItemStatus::Done => (theme::success(), ICON_STATUS_CHECK),
            ItemStatus::Failed => (theme::danger(), ICON_STATUS_X),
            ItemStatus::Unavailable => (theme::text_muted(), ICON_STATUS_X),
            ItemStatus::DrmSkipped => (theme::text_muted(), ICON_STATUS_SLASH),
            ItemStatus::Skipped => (theme::text_muted(), ICON_STATUS_SLASH),
            ItemStatus::Stopped => (theme::danger(), ICON_STATUS_SQUARE),
        }
    };

    let (source_icon, source_color) = match item.source {
        Source::YouTube => (ICON_SRC_YOUTUBE, Color::from_rgb(0.93, 0.20, 0.20)),
        Source::SoundCloud => (ICON_SRC_SOUNDCLOUD, Color::from_rgb(1.0, 0.33, 0.0)),
        Source::Spotify => (ICON_SRC_SPOTIFY, Color::from_rgb(0.114, 0.725, 0.329)),
        Source::Deezer => (ICON_SRC_DEEZER, Color::from_rgb(0.635, 0.220, 1.0)),
        Source::Unsupported => (ICON_STATUS_PENDING, theme::text_muted()),
    };

    let thumbnail: Element<'a, M> = match &item.thumbnail {
        ThumbState::Ready(handle) => image(handle.clone())
            .width(Length::Fixed(40.0))
            .height(Length::Fixed(40.0))
            .into(),
        ThumbState::Loading => container(
            svg(svg::Handle::from_memory(spinner_icon(tick)))
                .width(Length::Fixed(18.0))
                .height(Length::Fixed(18.0))
                .style(|_, _| svg::Style {
                    color: Some(theme::text_muted()),
                }),
        )
        .width(Length::Fixed(40.0))
        .height(Length::Fixed(40.0))
        .center_x(Length::Fixed(40.0))
        .center_y(Length::Fixed(40.0))
        .style(theme::thumbnail_placeholder)
        .into(),
        ThumbState::Failed => container(
            svg(svg::Handle::from_memory(ICON_THUMB_FAIL))
                .width(Length::Fixed(18.0))
                .height(Length::Fixed(18.0))
                .style(|_, _| svg::Style {
                    color: Some(theme::danger()),
                }),
        )
        .width(Length::Fixed(40.0))
        .height(Length::Fixed(40.0))
        .center_x(Length::Fixed(40.0))
        .center_y(Length::Fixed(40.0))
        .style(theme::thumbnail_placeholder)
        .into(),
    };

    let status_glyph = svg(svg::Handle::from_memory(status_icon))
        .width(Length::Fixed(14.0))
        .height(Length::Fixed(14.0))
        .style(move |_, _| svg::Style {
            color: Some(status_color),
        });

    let source_glyph = container(
        svg(svg::Handle::from_memory(source_icon))
            .width(Length::Fixed(12.0))
            .height(Length::Fixed(12.0))
            .style(move |_, _| svg::Style {
                color: Some(source_color),
            }),
    )
    .width(Length::Fixed(14.0))
    .height(Length::Fixed(16.0))
    .padding(iced::Padding {
        top: 3.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    })
    .center_x(Length::Fixed(14.0))
    .center_y(Length::Fixed(16.0));

    let dots = ".".repeat((tick / 4 % 3 + 1) as usize);
    let status_label: String = if is_header {
        let total = item.group.as_ref().map_or(0, |g| g.children.len());
        let done = item.group.as_ref().map_or(0, |g| g.children.iter().filter(|c| is_completed(c.status)).count());
        match effective_status {
            ItemStatus::Downloading => format!("Downloading \u{00B7} {done}/{total}"),
            ItemStatus::Done => format!("Done \u{00B7} {total} track{}", if total == 1 { "" } else { "s" }),
            ItemStatus::Failed => format!("Errors \u{00B7} {total} track{}", if total == 1 { "" } else { "s" }),
            _ => format!("Playlist \u{00B7} {total} track{}", if total == 1 { "" } else { "s" }),
        }
    } else if item.resolving {
        let is_playlist = match item.source {
            Source::Spotify => crate::backend::spotify::is_spotify_playlist_url(&item.url),
            Source::Deezer => crate::backend::deezer::is_deezer_playlist_url(&item.url),
            other => crate::backend::is_playlist_url(&item.url, other),
        };
        let what = if is_playlist {
            "Fetching playlist"
        } else {
            "Fetching track"
        };
        format!("{what}{dots:<3}")
    } else if (item.spotify.is_some() || item.deezer.is_some())
        && item.resolved_url.is_none()
        && item.status == ItemStatus::Pending
    {
        format!("Matching{dots:<3}")
    } else if matches!(item.thumbnail, ThumbState::Loading) && item.status == ItemStatus::Pending {
        format!("Fetching track{dots:<3}")
    } else {
        item.status.label().to_string()
    };

    let subtitle = row![
        source_glyph,
        text(format!("{} \u{00B7} {}", status_label, item.source.label()))
            .size(10)
            .style(move |_| iced::widget::text::Style {
                color: Some(status_color)
            }),
    ]
    .spacing(4)
    .align_y(Alignment::Center);

    let remove_btn = button(text("\u{00D7}").size(13))
        .on_press_maybe((!downloading).then_some(on_remove(item.id)))
        .style(theme::secondary_button)
        .padding([3, 8]);

    let (lead, name_limit): (Element<'a, M>, usize) = match kind {
        RowKind::Top(index) => (
            text(format!("{:>2}", index + 1))
                .size(11)
                .width(Length::Fixed(20.0))
                .style(theme::muted_text_strong)
                .into(),
            60,
        ),
        RowKind::Child => (Space::new().width(Length::Fixed(20.0)).into(), 54),
    };

    let mut row_content = row![
        lead,
        status_glyph,
        thumbnail,
        column![
            text(truncate(&item.display_name, name_limit)).size(12),
            subtitle,
        ]
        .spacing(2)
        .width(Length::Fill),
    ]
    .spacing(8)
    .padding([6, 8])
    .align_y(Alignment::Center);

    if let Some(dd) = dropdown {
        row_content = row_content.push(dropdown_button(item.id, dd, on_toggle));
    }
    row_content = row_content.push(remove_btn);

    let (style, height): (fn(&iced::Theme) -> container::Style, f32) = match kind {
        RowKind::Top(_) => (theme::card_elevated, 56.0),
        RowKind::Child => (theme::card_nested, 52.0),
    };

    container(row_content)
        .style(style)
        .width(Length::Fill)
        .height(Length::Fixed(height))
        .into()
}

fn dropdown_button<'a, M>(
    id: u64,
    dd: Dropdown,
    on_toggle: impl Fn(u64) -> M + 'a,
) -> Element<'a, M>
where
    M: Clone + 'a,
{
    let icon = if dd.expanded {
        ICON_CHEVRON_DOWN
    } else {
        ICON_CHEVRON_RIGHT
    };
    button(
        row![
            text(format!("{}/{}", dd.done, dd.total))
                .size(10)
                .style(theme::muted_text),
            svg(svg::Handle::from_memory(icon))
                .width(Length::Fixed(14.0))
                .height(Length::Fixed(14.0))
                .style(|_, _| svg::Style {
                    color: Some(theme::text_secondary())
                }),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    )
    .on_press(on_toggle(id))
    .style(theme::ghost_button)
    .padding([4, 6])
    .into()
}

fn spinner_icon(tick: u32) -> Vec<u8> {
    let angle = (tick % 8) * 45;
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2.5\" stroke-linecap=\"round\"><g transform=\"rotate({angle} 12 12)\"><path d=\"M21 12a9 9 0 1 1-6.219-8.56\"/></g></svg>"
    )
    .into_bytes()
}

pub fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}