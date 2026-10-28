use iced::theme::{Palette, Theme};
use iced::widget::{button, container, text, text_input};
use iced::{border::Radius, Background, Border, Color};

pub fn app_theme() -> Theme {
    Theme::custom(
        "Musicerer Mono Blue".to_string(),
        Palette {
            background: Color::TRANSPARENT,
            text: text_primary(),
            primary: primary(),
            success: success(),
            danger: danger(),
            warning: warning(),
        },
    )
}

pub fn bg() -> Color {
    Color::from_rgb(0.043, 0.043, 0.043)
}
pub fn surface() -> Color {
    Color::from_rgb(0.078, 0.078, 0.078)
}
pub fn surface_elevated() -> Color {
    Color::from_rgb(0.118, 0.118, 0.118)
}
/// Slightly lighter than `surface_elevated`, for rows nested inside a playlist dropdown.
pub fn surface_nested() -> Color {
    Color::from_rgb(0.169, 0.169, 0.169)
}
pub fn border_subtle() -> Color {
    Color::from_rgb(0.169, 0.169, 0.169)
}
pub fn border_strong() -> Color {
    Color::from_rgb8(0x42, 0x42, 0x42)
} // #424242
pub fn text_primary() -> Color {
    Color::from_rgb(0.902, 0.902, 0.902)
}
pub fn text_secondary() -> Color {
    Color::from_rgb(0.627, 0.627, 0.627)
}
pub fn text_muted() -> Color {
    Color::from_rgb(0.420, 0.420, 0.420)
}

pub fn primary() -> Color {
    Color::from_rgb(0.231, 0.510, 0.965)
}
pub fn success() -> Color {
    Color::from_rgb(0.400, 0.780, 0.500)
}
pub fn danger() -> Color {
    Color::from_rgb(0.900, 0.450, 0.450)
}
pub fn warning() -> Color {
    Color::from_rgb(0.950, 0.700, 0.350)
}

fn border(color: Color, width: f32, radius: f32) -> Border {
    Border {
        color,
        width,
        radius: radius.into(),
    }
}

pub fn topbar(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(surface())),
        border: Border {
            color: border_subtle(),
            width: 0.0,
            radius: Radius {
                top_left: 11.0,
                top_right: 11.0,
                bottom_left: 0.0,
                bottom_right: 0.0,
            },
        },
        ..Default::default()
    }
}

pub fn sidebar(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(surface())),
        border: border(Color::TRANSPARENT, 0.0, 0.0),
        ..Default::default()
    }
}

pub fn card(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(surface())),
        border: border(border_subtle(), 1.0, 6.0),
        ..Default::default()
    }
}

pub fn card_elevated(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(surface_elevated())),
        border: border(border_strong(), 1.0, 6.0),
        ..Default::default()
    }
}

pub fn card_nested(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(surface_nested())),
        border: border(border_strong(), 1.0, 6.0),
        ..Default::default()
    }
}

pub fn thumbnail_placeholder(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(surface())),
        border: border(border_subtle(), 1.0, 8.0),
        ..Default::default()
    }
}

pub fn status_bar(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(surface())),
        border: Border {
            color: border_subtle(),
            width: 0.0,
            radius: Radius {
                top_left: 0.0,
                top_right: 0.0,
                bottom_left: 11.0,
                bottom_right: 11.0,
            },
        },
        ..Default::default()
    }
}

pub fn muted_text(_: &Theme) -> text::Style {
    text::Style {
        color: Some(text_secondary()),
    }
}

pub fn muted_text_strong(_: &Theme) -> text::Style {
    text::Style {
        color: Some(text_muted()),
    }
}

pub fn text_input_style(_: &Theme, _status: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Background::Color(Color::TRANSPARENT),
        border: Border::default(),
        icon: text_secondary(),
        placeholder: text_muted(),
        value: text_primary(),
        selection: primary(),
    }
}

pub fn primary_button(_: &Theme, status: button::Status) -> button::Style {
    let base = button::Style {
        background: Some(Background::Color(primary())),
        text_color: Color::WHITE,
        border: border(primary(), 0.0, 6.0),
        ..Default::default()
    };
    match status {
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(Color::from_rgb(0.376, 0.647, 0.980))),
            ..base
        },
        button::Status::Pressed => button::Style {
            background: Some(Background::Color(Color::from_rgb(0.145, 0.388, 0.922))),
            ..base
        },
        button::Status::Disabled => button::Style {
            background: Some(Background::Color(Color::from_rgb(0.180, 0.180, 0.180))),
            text_color: text_muted(),
            ..base
        },
        _ => base,
    }
}

pub fn secondary_button(_: &Theme, status: button::Status) -> button::Style {
    let base = button::Style {
        background: Some(Background::Color(surface_elevated())),
        text_color: text_primary(),
        border: border(border_strong(), 1.0, 6.0),
        ..Default::default()
    };
    match status {
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(Color::from_rgb(0.180, 0.180, 0.180))),
            ..base
        },
        button::Status::Pressed => button::Style {
            background: Some(Background::Color(Color::from_rgb(0.140, 0.140, 0.140))),
            ..base
        },
        button::Status::Disabled => button::Style {
            background: Some(Background::Color(surface())),
            text_color: text_muted(),
            ..base
        },
        _ => base,
    }
}

/// Borderless button for small inline controls such as the playlist dropdown arrow.
pub fn ghost_button(_: &Theme, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(Background::Color(if hovered {
            Color::from_rgb(0.240, 0.240, 0.240)
        } else {
            Color::TRANSPARENT
        })),
        text_color: text_primary(),
        border: border(Color::TRANSPARENT, 0.0, 6.0),
        ..Default::default()
    }
}

pub fn nav_button(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let bg_color = if selected {
            surface_elevated()
        } else {
            Color::TRANSPARENT
        };
        let base = button::Style {
            background: Some(Background::Color(bg_color)),
            text_color: if selected {
                text_primary()
            } else {
                text_secondary()
            },
            border: if selected {
                border(primary(), 1.0, 6.0)
            } else {
                border(Color::TRANSPARENT, 0.0, 6.0)
            },
            ..Default::default()
        };
        match status {
            button::Status::Hovered if !selected => button::Style {
                background: Some(Background::Color(Color::from_rgb(0.110, 0.110, 0.110))),
                text_color: text_primary(),
                ..base
            },
            _ => base,
        }
    }
}

pub fn window_control_button(_: &Theme, status: button::Status) -> button::Style {
    let base = button::Style {
        background: Some(Background::Color(Color::TRANSPARENT)),
        text_color: text_primary(),
        border: Border::default(),
        ..Default::default()
    };
    match status {
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(Color::from_rgb(0.200, 0.200, 0.200))),
            text_color: text_primary(),
            ..base
        },
        button::Status::Pressed => button::Style {
            background: Some(Background::Color(Color::from_rgb(0.150, 0.150, 0.150))),
            text_color: text_primary(),
            ..base
        },
        _ => base,
    }
}

pub fn close_button(_: &Theme, status: button::Status) -> button::Style {
    let base = button::Style {
        background: Some(Background::Color(Color::TRANSPARENT)),
        text_color: text_primary(),
        border: Border::default(),
        ..Default::default()
    };
    match status {
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(Color::from_rgb(0.800, 0.200, 0.200))),
            text_color: Color::WHITE,
            border: Border {
                radius: Radius {
                    top_left: 0.0,
                    top_right: 11.0,
                    bottom_left: 0.0,
                    bottom_right: 0.0,
                },
                ..Default::default()
            },
            ..base
        },
        button::Status::Pressed => button::Style {
            background: Some(Background::Color(Color::from_rgb(0.600, 0.100, 0.100))),
            text_color: Color::WHITE,
            border: Border {
                radius: Radius {
                    top_left: 0.0,
                    top_right: 11.0,
                    bottom_left: 0.0,
                    bottom_right: 0.0,
                },
                ..Default::default()
            },
            ..base
        },
        _ => base,
    }
}
