#![windows_subsystem = "windows"]

mod app;
mod backend;
mod theme;
mod ui;

use app::App;

fn main() -> iced::Result {
    let icon = iced::window::icon::from_file("assets/icon.ico").ok();

    iced::application(App::new, App::update, App::view)
        .title("Musicerer")
        .theme(App::theme)
        .subscription(App::subscription)
        .window(iced::window::Settings {
            size: iced::Size::new(740.0, 480.0),
            min_size: Some(iced::Size::new(600.0, 480.0)),
            decorations: false,
            transparent: true,
            icon,
            ..Default::default()
        })
        .run()
}
