use crate::theme;
use iced::widget::{container, row};
use iced::{Background, Color, Element, Length};

const BAR_HEIGHT: f32 = 4.0;
const BAR_PARTS: f32 = 100.0;

/// Small, square-cornered progress bar built from two colored segments.
/// Avoiding rounded rectangles prevents a tiny-skia rounded-rect panic.
pub fn custom_progress_bar<'a, M: 'a>(progress: f32) -> Element<'a, M> {
    let progress = progress.clamp(0.0, 1.0);
    let filled = ((progress * BAR_PARTS).round() as u16).max(1);
    let remaining = (((1.0 - progress) * BAR_PARTS).round() as u16).max(1);

    row![
        segment(filled, theme::primary()),
        segment(remaining, theme::surface_elevated()),
    ]
    .width(Length::Fill)
    .height(Length::Fixed(BAR_HEIGHT))
    .into()
}

fn segment<'a, M: 'a>(portion: u16, color: Color) -> Element<'a, M> {
    container(iced::widget::text(""))
        .width(Length::FillPortion(portion))
        .height(Length::Fixed(BAR_HEIGHT))
        .style(move |_| container::Style {
            background: Some(Background::Color(color)),
            border: iced::Border {
                radius: 0.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}
