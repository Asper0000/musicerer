use super::{App, Message};
use crate::backend::vpn::VpnInfo;
use crate::theme as t;
use crate::ui::{
    custom_progress_bar,
    icons::{ICON_ALERT_TRIANGLE, ICON_NAV_DOWNLOADS, ICON_NAV_SETTINGS, ICON_SRC_SOUNDCLOUD, ICON_SRC_YOUTUBE},
    queue::queue_group,
    ItemStatus, UiLogLevel, View,
};

use iced::widget::{
    button, column, container, row, scrollable, svg, text, text_input, Column, MouseArea, Space, stack,
};
use iced::{Alignment, Element, Length, Color};
use iced::window::Direction;

const ICON_MINIMIZE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10"><line x1="1" y1="5" x2="9" y2="5" stroke="currentColor" stroke-width="1"/></svg>"#;
const ICON_MAXIMIZE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10"><rect x="1" y="1" width="8" height="8" fill="none" stroke="currentColor" stroke-width="1"/></svg>"#;
const ICON_CLOSE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10"><line x1="1" y1="1" x2="9" y2="9" stroke="currentColor" stroke-width="1"/><line x1="9" y1="1" x2="1" y2="9" stroke="currentColor" stroke-width="1"/></svg>"#;
const ICON_SVG: &[u8] = include_bytes!("../../assets/icon.svg");
const ICON_FOLDER: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>"#;
const ICON_DISK: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="4" width="20" height="16" rx="2" ry="2"/><line x1="6" y1="8" x2="6.01" y2="8"/><line x1="10" y1="8" x2="10.01" y2="8"/><line x1="14" y1="8" x2="14.01" y2="8"/><line x1="18" y1="8" x2="18.01" y2="8"/></svg>"#;
const ICON_PLUG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22v-5"/><path d="M9 8V2"/><path d="M15 8V2"/><path d="M18 8v5a4 4 0 0 1-4 4h-4a4 4 0 0 1-4-4V8Z"/></svg>"#;

fn resize_handle(dir: Direction, width: Length, height: Length) -> Element<'static, Message> {
    let interaction = match dir {
        Direction::North | Direction::South => iced::mouse::Interaction::ResizingVertically,
        Direction::East | Direction::West => iced::mouse::Interaction::ResizingHorizontally,
        Direction::NorthEast | Direction::SouthWest => iced::mouse::Interaction::ResizingDiagonallyUp,
        Direction::NorthWest | Direction::SouthEast => iced::mouse::Interaction::ResizingDiagonallyDown,
    };
    MouseArea::new(Space::new().width(width).height(height))
        .on_press(Message::WindowResize(dir))
        .interaction(interaction)
        .into()
}

impl App {
    pub fn view(&self) -> Element<'_, Message> {
        let sidebar = self.view_sidebar();
        let content = container(match self.current_view {
            View::Downloads => self.view_downloads(),
            View::Settings => self.view_settings(),
        })
        .padding(16)
        .width(Length::Fill)
        .height(Length::Fill);

        let top_separator = container(Space::new().height(1))
            .width(Length::Fill)
            .style(|_| iced::widget::container::Style {
                background: Some(iced::Background::Color(t::border_subtle())),
                ..Default::default()
            });

        let sidebar_separator = container(Space::new().width(1))
            .height(Length::Fill)
            .style(|_| iced::widget::container::Style {
                background: Some(iced::Background::Color(t::border_subtle())),
                ..Default::default()
            });

        let bottom_separator = container(Space::new().height(1))
            .width(Length::Fill)
            .style(|_| iced::widget::container::Style {
                background: Some(iced::Background::Color(t::border_subtle())),
                ..Default::default()
            });

        let content_area = row![sidebar, sidebar_separator, content].width(Length::Fill).height(Length::Fill);

        let content_with_overlay: Element<'_, Message> = if let Some(vpn) = &self.startup_vpn_warning {
            stack![content_area, self.view_vpn_overlay(vpn)].into()
        } else if self.show_platforms {
            stack![content_area, self.view_platforms_dialog()].into()
        } else {
            content_area.into()
        };

        let main_content = container(
            column![
                self.view_title_bar(),
                top_separator,
                content_with_overlay,
                bottom_separator,
                self.view_status_bar(),
            ]
        )
        .padding(1)
        .style(|_| iced::widget::container::Style {
            background: Some(iced::Background::Color(t::bg())),
            border: iced::Border {
                color: t::border_strong(),
                width: 1.0,
                radius: 12.0.into(),
            },
            ..Default::default()
        })
        .clip(true);

        let resize_overlay = column![
            row![
                resize_handle(Direction::NorthWest, Length::Fixed(8.0), Length::Fixed(8.0)),
                resize_handle(Direction::North, Length::Fill, Length::Fixed(8.0)),
                resize_handle(Direction::NorthEast, Length::Fixed(8.0), Length::Fixed(8.0)),
            ].height(8).width(Length::Fill),
            row![
                resize_handle(Direction::West, Length::Fixed(8.0), Length::Fill),
                Space::new().width(Length::Fill).height(Length::Fill),
                resize_handle(Direction::East, Length::Fixed(8.0), Length::Fill),
            ].width(Length::Fill).height(Length::Fill),
            row![
                resize_handle(Direction::SouthWest, Length::Fixed(8.0), Length::Fixed(8.0)),
                resize_handle(Direction::South, Length::Fill, Length::Fixed(8.0)),
                resize_handle(Direction::SouthEast, Length::Fixed(8.0), Length::Fixed(8.0)),
            ].height(8).width(Length::Fill),
        ].width(Length::Fill).height(Length::Fill);

        stack![main_content, resize_overlay].into()
    }

    fn view_title_bar(&self) -> Element<'_, Message> {
        let icon = svg(svg::Handle::from_memory(ICON_SVG))
            .width(Length::Fixed(22.0))
            .height(Length::Fixed(22.0));

        let title = text("Musicerer v0.2").size(13).style(t::muted_text_strong);

        let min_btn = button(
            container(svg(svg::Handle::from_memory(ICON_MINIMIZE)).width(Length::Fixed(12.0)).height(Length::Fixed(12.0)).style(|_, _| svg::Style { color: Some(t::text_primary()) }))
                .center_y(Length::Fill)
        )
        .on_press(Message::WindowMinimize)
        .style(t::window_control_button)
        .height(Length::Fixed(36.0))
        .padding([0, 16]);

        let max_btn = button(
            container(svg(svg::Handle::from_memory(ICON_MAXIMIZE)).width(Length::Fixed(12.0)).height(Length::Fixed(12.0)).style(|_, _| svg::Style { color: Some(t::text_primary()) }))
                .center_y(Length::Fill)
        )
        .on_press(Message::WindowMaximize)
        .style(t::window_control_button)
        .height(Length::Fixed(36.0))
        .padding([0, 16]);

        let close_btn = button(
            container(svg(svg::Handle::from_memory(ICON_CLOSE)).width(Length::Fixed(12.0)).height(Length::Fixed(12.0)).style(|_, _| svg::Style { color: Some(t::text_primary()) }))
                .center_y(Length::Fill)
        )
        .on_press(Message::WindowClose)
        .style(t::close_button)
        .height(Length::Fixed(36.0))
        .padding([0, 16]);

        let drag_content = row![icon, title]
            .spacing(10)
            .align_y(Alignment::Center);

        let drag_area = MouseArea::new(
            container(drag_content)
                .width(Length::Fill)
                .height(Length::Fill)
                .center_y(Length::Fill)
        ).on_press(Message::WindowDragged);

        container(
            row![drag_area, min_btn, max_btn, close_btn]
                .align_y(Alignment::Center)
                .height(Length::Fill)
        )
        .padding(iced::Padding { top: 0.0, right: 0.0, bottom: 0.0, left: 12.0 })
        .width(Length::Fill)
        .height(Length::Fixed(36.0))
        .style(t::topbar)
        .into()
    }

    fn view_platforms_dialog(&self) -> Element<'_, Message> {
        let sc_icon = svg(svg::Handle::from_memory(ICON_SRC_SOUNDCLOUD))
            .width(Length::Fixed(48.0))
            .height(Length::Fixed(48.0))
            .style(|_, _| svg::Style { color: Some(Color::from_rgb(1.0, 0.33, 0.0)) });

        let yt_icon = svg(svg::Handle::from_memory(ICON_SRC_YOUTUBE))
            .width(Length::Fixed(48.0))
            .height(Length::Fixed(48.0))
            .style(|_, _| svg::Style { color: Some(Color::from_rgb(0.93, 0.20, 0.20)) });

        let sp_icon = svg(svg::Handle::from_memory(crate::ui::icons::ICON_SRC_SPOTIFY))

            .width(Length::Fixed(48.0))

            .height(Length::Fixed(48.0))

            .style(|_, _| svg::Style { color: Some(Color::from_rgb(0.114, 0.725, 0.329)) });

        let platforms_row = row![
            column![sc_icon, Space::new().height(8), text("SoundCloud").size(12)].align_x(Alignment::Center),
            Space::new().width(40),
            column![yt_icon, Space::new().height(8), text("YouTube").size(12)].align_x(Alignment::Center),
            Space::new().width(40),
            column![sp_icon, Space::new().height(8), text("Spotify").size(12)].align_x(Alignment::Center),
        ].align_y(Alignment::Center);

        let card = container(
            column![
                text("Supported Platforms").size(18).width(Length::Fill).align_x(Alignment::Center),
                Space::new().height(12),
                text("Musicerer currently supports downloading from these services:")
                    .size(13)
                    .style(t::muted_text)
                    .width(Length::Fill)
                    .align_x(Alignment::Center),
                Space::new().height(24),
                container(platforms_row).width(Length::Fill).center_x(Length::Fill),
                Space::new().height(28),
                row![
                    Space::new().width(Length::Fill),
                    button(text("Close").size(13))
                        .on_press(Message::TogglePlatformsDialog)
                        .style(t::primary_button)
                        .padding([10, 24]),
                ],
            ]
            .spacing(0),
        )
        .style(t::card_elevated)
        .padding(28)
        .width(Length::Fixed(400.0));

        container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(|_| iced::widget::container::Style {
                background: Some(iced::Background::Color(Color { r: 0.0, g: 0.0, b: 0.0, a: 0.75 })),
                ..Default::default()
            })
            .into()
    }

    fn view_vpn_overlay(&self, vpn: &VpnInfo) -> Element<'_, Message> {
        let alert = svg(svg::Handle::from_memory(ICON_ALERT_TRIANGLE))
            .width(Length::Fixed(28.0))
            .height(Length::Fixed(28.0))
            .style(|_, _| svg::Style { color: Some(t::warning()) });

        let card = container(
            column![
                row![alert, text("VPN detected").size(18)]
                    .spacing(12)
                    .align_y(Alignment::Center),
                Space::new().height(14),
                text(format!(
                    "{} is currently running on your computer.",
                    vpn.display_name()
                ))
                .size(13),
                Space::new().height(8),
                text(
                    "Some VPN servers cause YouTube to block video downloads. \
                     If downloads fail with \"Video unavailable\", try a different \
                     VPN server or disconnect the VPN."
                )
                .size(12)
                .style(t::muted_text),
                Space::new().height(20),
                row![
                    Space::new().width(Length::Fill),
                    button(text("Got it").size(13))
                        .on_press(Message::DismissVpnWarning)
                        .style(t::primary_button)
                        .padding([10, 24]),
                ],
            ]
            .spacing(0),
        )
        .style(t::card_elevated)
        .padding(28)
        .width(Length::Fixed(460.0));

        container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(|_| iced::widget::container::Style {
                background: Some(iced::Background::Color(Color { r: 0.0, g: 0.0, b: 0.0, a: 0.75 })),
                ..Default::default()
            })
            .into()
    }

    fn view_sidebar(&self) -> Element<'_, Message> {
        let folder_name = std::path::Path::new(&self.output_dir)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Musicerer");

        let folder_row = button(
            row![
                svg(svg::Handle::from_memory(ICON_FOLDER))
                    .width(Length::Fixed(14.0))
                    .height(Length::Fixed(14.0))
                    .style(|_,_| svg::Style { color: Some(Color::from_rgb(0.95, 0.80, 0.30)) }),
                text(folder_name).size(12).style(|_| iced::widget::text::Style { color: Some(t::text_primary()) })
            ].spacing(8).align_y(Alignment::Center)
        )
        .on_press(Message::OpenOutputDir)
        .style(|_, _| iced::widget::button::Style {
            background: Some(iced::Background::Color(Color::TRANSPARENT)),
            text_color: Color::WHITE,
            border: iced::Border::default(),
            ..Default::default()
        })
        .padding(0);

        let disk_row = row![
            svg(svg::Handle::from_memory(ICON_DISK))
                .width(Length::Fixed(14.0))
                .height(Length::Fixed(14.0))
                .style(|_,_| svg::Style { color: Some(t::text_secondary()) }),
            text(&self.available_space).size(12).style(t::muted_text)
        ].spacing(8).align_y(Alignment::Center);

        let info_widget = column![
            folder_row,
            Space::new().height(6),
            disk_row
        ].padding(iced::Padding { top: 16.0, right: 14.0, bottom: 0.0, left: 14.0 });

        let separator = container(Space::new().height(1))
            .width(Length::Fill)
            .style(|_| iced::widget::container::Style {
                background: Some(iced::Background::Color(t::border_subtle())),
                ..Default::default()
            });

        let downloads_btn = button(
            row![
                svg(svg::Handle::from_memory(ICON_NAV_DOWNLOADS))
                    .width(Length::Fixed(16.0))
                    .height(Length::Fixed(16.0))
                    .style(move |_, _| svg::Style {
                        color: Some(if self.current_view == View::Downloads {
                            t::text_primary()
                        } else {
                            t::text_secondary()
                        }),
                    }),
                text("Downloads").size(13),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .on_press(Message::SwitchView(View::Downloads))
        .style(t::nav_button(self.current_view == View::Downloads))
        .width(Length::Fill)
        .padding([8, 10]);

        let settings_btn = button(
            row![
                svg(svg::Handle::from_memory(ICON_NAV_SETTINGS))
                    .width(Length::Fixed(16.0))
                    .height(Length::Fixed(16.0))
                    .style(move |_, _| svg::Style {
                        color: Some(if self.current_view == View::Settings {
                            t::text_primary()
                        } else {
                            t::text_secondary()
                        }),
                    }),
                text("Settings").size(13),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .on_press(Message::SwitchView(View::Settings))
        .style(t::nav_button(self.current_view == View::Settings))
        .width(Length::Fill)
        .padding([8, 10]);

        let nav = column![downloads_btn, settings_btn]
            .spacing(6)
            .padding([0, 8]);

        container(column![info_widget, separator, nav].spacing(14).height(Length::Fill))
            .style(t::sidebar)
            .width(Length::Fixed(190.0))
            .height(Length::Fill)
            .into()
    }

    fn view_downloads(&self) -> Element<'_, Message> {
        let header = row![
            text("Downloads").size(20),
            Space::new().width(Length::Fill),
            button(
                svg(svg::Handle::from_memory(ICON_PLUG))
                    .width(Length::Fixed(14.0))
                    .height(Length::Fixed(14.0))
                    .style(|_, _| svg::Style { color: Some(Color::WHITE) })
            )
            .on_press(Message::TogglePlatformsDialog)
            .style(t::primary_button)
            .padding(6)
        ].align_y(Alignment::Center);

        let url_input = text_input(
            "Input music URL",
            &self.url_input,
        )
        .on_input(Message::UrlChanged)
        .on_submit(Message::AddUrl)
        .padding(10)
        .size(13)
        .style(t::text_input_style);

        let add_btn = button(text("Add").size(13))
            .on_press_maybe(
                (!self.downloading && !self.url_input.trim().is_empty())
                    .then_some(Message::AddUrl),
            )
            .style(t::primary_button)
            .padding([10, 16]);

        let url_card = container(row![url_input, add_btn].spacing(6).align_y(Alignment::Center))
            .style(t::card)
            .padding(10)
            .width(Length::Fill);

        let track_count = self.tracks().count();
        let resolving = self.queue.iter().any(|i| i.resolving);

        let queue_header = row![
            text("Queue").size(14),
            Space::new().width(Length::Fill),
            text(format!(
                "{} track{}",
                track_count,
                if track_count == 1 { "" } else { "s" }
            ))
            .size(11)
            .style(t::muted_text),
        ]
        .align_y(Alignment::Center);

        let queue_rows: Vec<Element<'_, Message>> = if self.queue.is_empty() {
            vec![container(
                column![
                    text("No tracks yet").size(13).style(t::muted_text),
                    text("Paste a URL above and click Add.")
                        .size(11)
                        .style(t::muted_text_strong),
                ]
                .spacing(3)
                .align_x(Alignment::Center),
            )
            .padding(16)
            .width(Length::Fill)
            .center_x(Length::Fill)
            .into()]
        } else {
            self.queue
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    queue_group(i, item, self.downloading, Message::RemoveTrack, Message::ToggleGroup, self.tick)
                })
                .collect()
        };

        let queue_list = scrollable(Column::with_children(queue_rows).spacing(4))
            .height(Length::Fill);

        let queue_card = container(column![queue_header, queue_list].spacing(6))
            .style(t::card)
            .padding(10)
            .width(Length::Fill)
            .height(Length::Fill);

        let download_all = button(text("Download All").size(13))
            .on_press_maybe(
                (!self.downloading && !self.queue.is_empty() && !resolving)
                    .then_some(Message::StartQueue),
            )
            .style(t::primary_button)
            .padding([8, 14]);

        let stop = button(text("Stop").size(13))
            .on_press_maybe(self.downloading.then_some(Message::StopQueue))
            .style(t::secondary_button)
            .padding([8, 14]);

        let clear = button(text("Clear").size(13))
            .on_press_maybe(
                (!self.downloading && !self.queue.is_empty()).then_some(Message::ClearQueue),
            )
            .style(t::secondary_button)
            .padding([8, 14]);

        let log_toggle = {
            let label = text(if self.show_log { "Hide Log" } else { "Show Log" }).size(13);
            let btn = button(label).on_press(Message::ToggleLog).padding([8, 12]);
            if self.log_attention > 0 {
                btn.style(t::primary_button)
            } else {
                btn.style(t::secondary_button)
            }
        };

        let controls = row![
            download_all,
            stop,
            clear,
            Space::new().width(Length::Fill),
            log_toggle,
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        let progress = custom_progress_bar(self.overall_progress());

        let mut content = Column::new()
            .spacing(10)
            .width(Length::Fill)
            .height(Length::Fill);

        content = content.push(header);
        content = content.push(url_card);
        content = content.push(queue_card);
        content = content.push(controls);
        if self.downloading {
            content = content.push(progress);
        }

        if self.show_log {
            content = content.push(self.view_log_panel());
        }

        content.into()
    }

    fn view_log_panel(&self) -> Element<'_, Message> {
        let log_rows: Vec<Element<'_, Message>> = if self.logs.is_empty() {
            vec![text("Nothing yet. Start a download to see activity here.")
                .size(11)
                .style(t::muted_text)
                .into()]
        } else {
            self.logs
                .iter()
                .map(|line| {
                    let color = match line.level {
                        UiLogLevel::Info => t::text_secondary(),
                        UiLogLevel::Warn => t::warning(),
                        UiLogLevel::Error => t::danger(),
                    };
                    text(line.text.as_str())
                        .size(11)
                        .style(move |_| iced::widget::text::Style { color: Some(color) })
                        .into()
                })
                .collect()
        };
        let log_list = scrollable(Column::with_children(log_rows).spacing(2))
            .height(Length::Fill);
        container(column![text("Log").size(14), log_list].spacing(6))
            .style(t::card)
            .padding(10)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_settings(&self) -> Element<'_, Message> {
        let header = text("Settings").size(20);

        let output_input = text_input("Output folder\u{2026}", &self.output_dir)
            .on_input(Message::OutputDirChanged)
            .padding(10)
            .size(13)
            .width(Length::Fill)
            .style(t::text_input_style);

        let output_browse = button(text("Browse").size(13))
            .on_press_maybe((!self.downloading).then_some(Message::BrowseOutputDir))
            .style(t::secondary_button)
            .padding([10, 14]);

        let output_row = container(row![output_input, output_browse].spacing(6).align_y(Alignment::Center))
            .style(t::card)
            .padding(10)
            .width(Length::Fill);

        let output_section = column![
            text("Output folder").size(13),
            text("Finished .m4a files are saved here.")
                .size(11)
                .style(t::muted_text),
            Space::new().height(6),
            output_row,
        ]
        .spacing(3);

        let cookies_input = text_input("Optional cookies.txt", &self.cookies_input)
            .on_input(Message::CookiesChanged)
            .padding(10)
            .size(13)
            .width(Length::Fill)
            .style(t::text_input_style);

        let cookies_browse = button(text("Browse").size(13))
            .on_press_maybe((!self.downloading).then_some(Message::BrowseCookies))
            .style(t::secondary_button)
            .padding([10, 14]);

        let cookies_row = container(row![cookies_input, cookies_browse].spacing(6).align_y(Alignment::Center))
            .style(t::card)
            .padding(10)
            .width(Length::Fill);

        let cookies_section = column![
            text("Cookies file").size(13),
            text("Netscape-format cookies.txt for age-gated or private YouTube content.")
                .size(11)
                .style(t::muted_text),
            Space::new().height(6),
            cookies_row,
        ]
        .spacing(3);


        let group_on = self.group_playlists;
        let group_btn = button(
            text(if group_on { "Enabled" } else { "Disabled" })
                .size(13)
                .width(Length::Fill)
                .align_x(Alignment::Center),
        )
        .on_press(Message::ToggleGroupPlaylists(!group_on))
        .width(Length::Fixed(90.0))
        .padding([8, 14]);
        let group_btn = if group_on {
            group_btn.style(t::primary_button)
        } else {
            group_btn.style(t::secondary_button)
        };

        let group_section = row![
            column![
                text("Playlist folders").size(13),
                text("Save playlist and album tracks into their own subfolder of the output folder.")
                    .size(11)
                    .style(t::muted_text),
            ]
            .spacing(3)
            .width(Length::Fill),
            group_btn,
        ]
        .spacing(12)
        .align_y(Alignment::Center);

        let settings_card = container(
            column![
                output_section,
                Space::new().height(12),
                cookies_section,
                Space::new().height(12),
                group_section,
                Space::new().height(12),
            ]
            .spacing(3),
        )
        .style(t::card)
        .padding(14)
        .width(Length::Fill);

        column![
            header,
            Space::new().height(6),
            scrollable(settings_card).height(Length::Fill),
        ]
        .spacing(6)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn view_status_bar(&self) -> Element<'_, Message> {
        let status_text = text(self.status.as_str()).size(11).style(t::muted_text);
        let done = self.tracks().filter(|i| i.status == ItemStatus::Done).count();
        let unavailable = self.tracks().filter(|i| i.status == ItemStatus::Unavailable).count();
        let bad = self
            .tracks()
            .filter(|i| matches!(i.status, ItemStatus::Failed | ItemStatus::Stopped))
            .count();
        let counts = text(format!(
            "{} tracks  |  {} done  |  {} unavailable  |  {} failed",
            self.tracks().count(),
            done,
            unavailable,
            bad
        ))
        .size(11)
        .style(t::muted_text_strong);

        container(row![status_text, Space::new().width(Length::Fill), counts])
            .style(t::status_bar)
            .padding([6, 12])
            .width(Length::Fill)
            .into()
    }
}
