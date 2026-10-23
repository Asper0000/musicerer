use crate::backend::Source;
use crate::theme;
use crate::ui::{
    icons::*,
    ItemStatus, QueueItem, ThumbState,
};
use iced::widget::{button, column, container, image, row, svg, text};
use iced::{Alignment, Color, Element, Length};

pub fn queue_row<'a, M: 'a>(
    index: usize,
    item: &'a QueueItem,
    downloading: bool,
    on_remove: impl Fn(u64) -> M + 'a,
) -> Element<'a, M>
where
    M: Clone + 'a,
{
    let (status_color, status_icon) = match item.status {
        ItemStatus::Pending => (theme::text_muted(), ICON_STATUS_PENDING),
        ItemStatus::Downloading => (theme::primary(), ICON_STATUS_DOWNLOADING),
        ItemStatus::Done => (theme::success(), ICON_STATUS_CHECK),
        ItemStatus::Failed => (theme::danger(), ICON_STATUS_X),
        ItemStatus::DrmSkipped => (theme::text_muted(), ICON_STATUS_SLASH),
        ItemStatus::Stopped => (theme::danger(), ICON_STATUS_SQUARE),
    };

    let (source_icon, source_color) = match item.source {
        Source::YouTube => (ICON_SRC_YOUTUBE, Color::from_rgb(0.93, 0.20, 0.20)),
        Source::SoundCloud => (ICON_SRC_SOUNDCLOUD, Color::from_rgb(1.0, 0.33, 0.0)),
        Source::Unsupported => (ICON_STATUS_PENDING, theme::text_muted()),
    };

    let thumbnail: Element<'a, M> = match &item.thumbnail {
        ThumbState::Ready(handle) => image(handle.clone())
            .width(Length::Fixed(40.0))
            .height(Length::Fixed(40.0))
            .into(),
        ThumbState::Loading => container(
            svg(svg::Handle::from_memory(ICON_STATUS_PENDING))
                .width(Length::Fixed(18.0))
                .height(Length::Fixed(18.0))
                .style(|_, _| svg::Style { color: Some(theme::text_muted()) }),
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
                .style(|_, _| svg::Style { color: Some(theme::danger()) }),
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
        .style(move |_, _| svg::Style { color: Some(status_color) });

    let source_glyph = svg(svg::Handle::from_memory(source_icon))
        .width(Length::Fixed(12.0))
        .height(Length::Fixed(12.0))
        .style(move |_, _| svg::Style { color: Some(source_color) });

    let subtitle = row![
        source_glyph,
        text(format!("{} \u{00B7} {}", item.status.label(), item.source.label()))
            .size(10)
            .style(move |_| iced::widget::text::Style { color: Some(status_color) }),
    ]
    .spacing(4)
    .align_y(Alignment::Center);

    let remove_btn = button(text("\u{00D7}").size(13))
        .on_press_maybe((!downloading).then_some(on_remove(item.id)))
        .style(theme::secondary_button)
        .padding([3, 8]);

    let row_content = row![
        text(format!("{:>2}", index + 1))
            .size(11)
            .width(Length::Fixed(20.0))
            .style(theme::muted_text_strong),
        status_glyph,
        thumbnail,
        column![
            text(truncate(&item.display_name, 60)).size(12),
            subtitle,
        ]
        .spacing(2)
        .width(Length::Fill),
        text(format!("{:.0}%", item.progress * 100.0))
            .size(11)
            .width(Length::Fixed(36.0))
            .style(theme::muted_text),
        remove_btn,
    ]
    .spacing(8)
    .padding([6, 8])
    .align_y(Alignment::Center);

    container(row_content)
        .style(theme::card_elevated)
        .width(Length::Fill)
        .height(Length::Fixed(56.0))
        .into()
}

pub fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}