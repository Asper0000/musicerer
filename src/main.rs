#![windows_subsystem = "windows"]

mod app;
mod backend;
mod theme;
mod ui;

use app::App;

const WINDOW_ICON: &[u8] = include_bytes!("../assets/icon.ico");

/// Checks for the external tools off the UI thread and, if any required one is
/// missing, shows a native warning. The app itself opens regardless.
fn warn_about_missing_tools() {
    std::thread::spawn(|| {
        if let Some(text) = backend::tools::missing_report() {
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("Musicerer - missing tools")
                .set_description(text)
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
        }
    });
}

fn main() -> iced::Result {
    backend::cleanup_stale_temp_dirs();
    warn_about_missing_tools();

    let icon = iced::window::icon::from_file_data(WINDOW_ICON, None).ok();

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
