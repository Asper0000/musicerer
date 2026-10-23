use crate::theme;
use iced::widget::{container, row};
use iced::{Background, Element, Length};

/// Two-container bar: filled portion + remaining portion.
/// No rounded corners => cannot trigger the tiny-skia rounded-rect panic.
pub fn custom_progress_bar<'a, M: 'a>(progress: f32) -> Element<'a, M> {
    let p = progress.clamp(0.0, 1.0);
    let filled_portion = ((p * 100.0).round() as u16).max(1);
    let remaining_portion = (((1.0 - p) * 100.0).round() as u16).max(1);

    let filled = container(container(iced::widget::text("")).height(Length::Fixed(4.0)))
        .width(Length::FillPortion(filled_portion))
        .height(Length::Fixed(4.0))
        .style(|_| container::Style {
            background: Some(Background::Color(theme::primary())),
            border: iced::Border { radius: 0.0.into(), ..Default::default() },
            ..Default::default()
        });

    let remaining = container(container(iced::widget::text("")).height(Length::Fixed(4.0)))
        .width(Length::FillPortion(remaining_portion))
        .height(Length::Fixed(4.0))
        .style(|_| container::Style {
            background: Some(Background::Color(theme::surface_elevated())),
            border: iced::Border { radius: 0.0.into(), ..Default::default() },
            ..Default::default()
        });

    row![filled, remaining]
        .width(Length::Fill)
        .height(Length::Fixed(4.0))
        .into()
}