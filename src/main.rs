#![windows_subsystem = "windows"]

mod app;
mod backend;
mod theme;
mod ui;

use app::App;
use std::path::PathBuf;

fn main() -> iced::Result {
    let icon_path = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("assets").join("icon.ico")))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("assets/icon.ico"));
    let icon = iced::window::icon::from_file(icon_path).ok();

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
