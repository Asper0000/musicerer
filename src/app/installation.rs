//! UI for application and dependency installation management.

use super::{App, Message};
use crate::backend::updater::{self, Component, Status};
use crate::theme as t;
use crate::ui::{
    custom_progress_bar,
    icons::{
        ICON_ACTION_CANCEL, ICON_ACTION_CONFIRM, ICON_ACTION_INSTALL, ICON_ACTION_REINSTALL,
        ICON_ACTION_UNINSTALL, ICON_ALERT_TRIANGLE, ICON_HELP,
    },
};

use iced::widget::{
    button, checkbox, column, container, row, scrollable, svg, tooltip, Column, Row, Space, text,
};
use iced::{Alignment, Background, Border, Color, Element, Length};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn action_tooltip<'a>(content: impl Into<Element<'a, Message>>, label: &'static str) -> Element<'a, Message> {
    tooltip(
        content,
        container(text(label).size(11))
            .style(t::card_elevated)
            .padding([6, 9]),
        iced::widget::tooltip::Position::Left,
    )
    .delay(Duration::from_millis(350))
    .gap(6)
    .into()
}

fn square_action_button(
    icon: &'static [u8],
    label: &'static str,
    message: Message,
    enabled: bool,
    primary: bool,
) -> Element<'static, Message> {
    let style = if primary { t::primary_button } else { t::secondary_button };
    action_tooltip(
        button(
            svg(svg::Handle::from_memory(icon))
                .width(Length::Fixed(16.0))
                .height(Length::Fixed(16.0))
                .style(move |_, _| svg::Style {
                    color: Some(if enabled { t::text_primary() } else { Color { a: 0.38, ..t::text_primary() } }),
                }),
        )
        .on_press_maybe(enabled.then_some(message))
        .style(style)
        .padding(8)
        .width(Length::Fixed(36.0))
        .height(Length::Fixed(36.0)),
        label,
    )
}

fn installation_info_entry(
    label: &'static str,
    displayed_path: String,
    open_path: Option<PathBuf>,
) -> Element<'static, Message> {
    let content = column![
        text(label).size(12),
        text(displayed_path).size(11).style(t::muted_text),
    ]
    .spacing(2)
    .width(Length::Fill);

    if let Some(path) = open_path {
        button(content)
            .on_press(Message::OpenInstallationDirectory(path))
            .style(t::ghost_button)
            .padding([5, 8])
            .width(Length::Fill)
            .into()
    } else {
        content.into()
    }
}

fn displayed_path(path: &Option<PathBuf>) -> String {
    path.as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "Unavailable".to_string())
}

fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// Shows the current installed footprint followed by the expected space
/// change. This intentionally remains based on the checked release plan while
/// downloading/installing, so the label does not jump when progress begins.
fn tool_size_description(r: &updater::Row) -> Option<String> {
    let current = if r.managed {
        r.managed_size.or(r.installed_size)
    } else {
        r.installed_size
    }
    .filter(|size| *size > 0);

    // A PATH installation is never replaced. Its existing footprint and the
    // size of the separate Musicerer-managed copy therefore coexist, even
    // before the user starts downloading that managed copy.
    if r.external && !r.managed {
        let managed_target = r.latest_size.filter(|size| *size > 0);
        return match (current, managed_target) {
            (Some(current), Some(managed_target)) => Some(format!(
                "{} + {}",
                format_size(current),
                format_size(managed_target)
            )),
            // If the external installation cannot be measured reliably (for
            // example, a yt-dlp PATH launcher whose owning Python environment
            // cannot be identified), still show the known size of the separate
            // Musicerer-managed copy that will be downloaded.
            (None, Some(managed_target)) => Some(format!("? MB + {}", format_size(managed_target))),
            (Some(current), None) => Some(format_size(current)),
            (None, None) => None,
        };
    }

    let update_in_progress = r.update_available
        && matches!(
            &r.status,
            Status::Available | Status::Downloading(_) | Status::Installing
        );

    if update_in_progress {
        if let Some(target) = r.latest_size.filter(|size| *size > 0) {
            if let Some(current) = current {
                let delta = target as i128 - current as i128;
                return Some(if delta > 0 {
                    format!("{} + {}", format_size(current), format_size(delta as u64))
                } else if delta < 0 {
                    format!("{} − {}", format_size(current), format_size((-delta) as u64))
                } else {
                    format_size(current)
                });
            }
            return Some(format!("? MB + {}", format_size(target)));
        }
    }

    current.map(format_size)
}

fn status_tag(
    label: &'static str,
    icon: Option<&'static [u8]>,
    detail: String,
    accent: Color,
) -> Element<'static, Message> {
    let mut contents = Row::new().spacing(4).align_y(Alignment::Center);
    if let Some(icon) = icon {
        contents = contents.push(
            svg(svg::Handle::from_memory(icon))
                .width(Length::Fixed(11.0))
                .height(Length::Fixed(11.0))
                .style(move |_, _| svg::Style { color: Some(accent) }),
        );
    }
    contents = contents.push(
        text(label)
            .size(10)
            .style(move |_| iced::widget::text::Style { color: Some(accent) }),
    );

    let tag = container(contents)
        .padding([3, 8])
        .style(move |_| iced::widget::container::Style {
            background: Some(Background::Color(Color {
                a: 0.10,
                ..accent
            })),
            border: Border {
                color: Color { a: 0.22, ..accent },
                width: 1.0,
                radius: 99.0.into(),
            },
            ..Default::default()
        });

    tooltip(
        tag,
        container(text(detail).size(11))
            .style(t::card_elevated)
            .padding([6, 9]),
        iced::widget::tooltip::Position::Top,
    )
    .delay(Duration::from_millis(300))
    .gap(5)
    .into()
}

fn dialog_close_button(message: Message) -> Element<'static, Message> {
    button(
        svg(svg::Handle::from_memory(ICON_ACTION_CANCEL))
            .width(Length::Fixed(16.0))
            .height(Length::Fixed(16.0))
            .style(|_, _| svg::Style { color: Some(t::text_primary()) }),
    )
    .on_press(message)
    .style(t::secondary_button)
    .padding(8)
    .width(Length::Fixed(34.0))
    .height(Length::Fixed(34.0))
    .into()
}

fn help_section_heading(label: &'static str) -> Element<'static, Message> {
    let separator = container(Space::new().height(1))
        .width(Length::Fill)
        .style(|_| iced::widget::container::Style {
            background: Some(Background::Color(t::border_subtle())),
            ..Default::default()
        });
    column![text(label).size(14), separator]
        .spacing(5)
        .width(Length::Fill)
        .into()
}

fn help_tool_row(name: &'static str, description: &'static str) -> Element<'static, Message> {
    row![
        text(name).size(12).style(|_| iced::widget::text::Style { color: Some(t::primary()) }),
        text(description).size(12),
    ]
    .spacing(9)
    .align_y(Alignment::Center)
    .into()
}

impl App {
    pub(super) fn view_installation(&self) -> Element<'_, Message> {
        let rows = updater::snapshot(&self.updater);
        let busy = updater::is_busy(&self.updater);
        let queue_busy = self.downloading || self.is_loading();
        let app_row = rows
            .iter()
            .find(|(comp, _)| *comp == Component::App)
            .map(|(_, row)| row.clone())
            .unwrap_or_else(updater::Row::empty);

        let update_button = button(text("Check for updates").size(13))
            .on_press_maybe((!busy).then_some(Message::CheckUpdates))
            .style(t::update_check_button)
            .padding([8, 14])
            .height(Length::Fixed(36.0));
        let help_button = action_tooltip(
            button(
                svg(svg::Handle::from_memory(ICON_HELP))
                    .width(Length::Fixed(10.0))
                    .height(Length::Fixed(10.0))
                    .style(|_, _| svg::Style { color: Some(t::text_primary()) }),
            )
            .on_press(Message::ToggleToolInfoDialog)
            .style(t::secondary_button)
            .padding(0)
            .width(Length::Fixed(36.0))
            .height(Length::Fixed(36.0)),
            "How tools are managed",
        );
        let header = row![
            text("Installation").size(20),
            Space::new().width(Length::Fill),
            row![update_button, help_button].spacing(6),
        ]
        .align_y(Alignment::Center);

        let mut cards: Vec<Element<'_, Message>> = Vec::new();
        cards.push(self.application_card(app_row, busy, queue_busy));
        cards.push(text("Tools").size(15).into());
        for (comp, row) in rows {
            if comp != Component::App {
                cards.push(self.tool_management_card(comp, row, busy, queue_busy));
            }
        }
        if updater::any_external(&self.updater) {
            cards.push(self.external_tools_card(busy, queue_busy));
        }
        cards.push(text("Installation information").size(15).into());
        cards.push(self.installation_information_card());

        if queue_busy {
            cards.push(
                text("Installation actions are paused while downloads are running.")
                    .size(11)
                    .style(t::muted_text)
                    .into(),
            );
        }
        let list = Column::with_children(cards).spacing(10).width(Length::Fill);

        column![
            header,
            Space::new().height(6),
            scrollable(list).height(Length::Fill),
        ]
        .spacing(6)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn application_card(
        &self,
        r: updater::Row,
        busy: bool,
        queue_busy: bool,
    ) -> Element<'_, Message> {
        let installed = r.installed.clone().unwrap_or_else(|| format!("v{}", env!("CARGO_PKG_VERSION")));
        let version = if r.status == Status::Available {
            format!("{}  →  {}", installed, r.latest.clone().unwrap_or_default())
        } else {
            installed
        };
        let status_text = match &r.status {
            Status::Unchecked => "Update status not checked".to_string(),
            Status::Checking => "Checking for updates…".to_string(),
            Status::Downloading(p) => format!("Downloading {:.0}%", *p * 100.0),
            Status::Installing => "Installing and restarting…".to_string(),
            Status::Removing => "Removing…".to_string(),
            Status::Failed(error) => error.clone(),
            Status::Unsupported(message) => message.clone(),
            Status::UpToDate => "Up to date".to_string(),
            Status::Available if self.confirm_app_update => {
                "Confirm to restart and update".to_string()
            }
            Status::Available => "Update available".to_string(),
        };
        let status_color = match &r.status {
            Status::Failed(_) => Some(t::danger()),
            Status::Available => Some(t::warning()),
            Status::UpToDate => Some(t::success()),
            _ => None,
        };
        let version_widget = text(version).size(12);
        let version_widget = if r.status == Status::Available {
            version_widget.style(|_| iced::widget::text::Style { color: Some(t::warning()) })
        } else if r.status == Status::UpToDate {
            version_widget.style(|_| iced::widget::text::Style { color: Some(t::success()) })
        } else {
            version_widget.style(t::muted_text)
        };
        let status_widget = text(status_text).size(11);
        let status_widget = if let Some(color) = status_color {
            status_widget.style(move |_| iced::widget::text::Style { color: Some(color) })
        } else {
            status_widget.style(t::muted_text)
        };

        let mut info = column![
            text("Musicerer").size(14),
            version_widget,
            status_widget,
            text(format!("Installation type: {}", self.installation_info.kind)).size(11).style(t::muted_text),
        ]
        .spacing(4)
        .width(Length::Fill);
        if let Some(package) = &self.installation_info.package {
            info = info.push(text(format!("Package: {package}")).size(11).style(t::muted_text));
        }
        if !self.installation_info.uninstall_note.is_empty() && !self.installation_info.can_uninstall {
            info = info.push(text(self.installation_info.uninstall_note.clone()).size(11).style(t::muted_text));
        }

        let mut actions = Column::new()
            .spacing(5)
            .width(Length::Fixed(36.0))
            .align_x(Alignment::End);
        if self.confirm_app_update {
            actions = actions
                .push(square_action_button(
                    ICON_ACTION_CANCEL,
                    "Cancel",
                    Message::CancelAppUpdate,
                    true,
                    false,
                ))
                .push(square_action_button(
                    ICON_ACTION_CONFIRM,
                    "Restart & update",
                    Message::InstallUpdate(Component::App),
                    !busy && !queue_busy,
                    true,
                ));
        } else if self.confirm_app_reinstall {
            actions = actions
                .push(square_action_button(
                    ICON_ACTION_CANCEL,
                    "Cancel",
                    Message::CancelAppReinstall,
                    true,
                    false,
                ))
                .push(square_action_button(
                    ICON_ACTION_CONFIRM,
                    "Confirm",
                    Message::InstallAppReinstall,
                    !busy && !queue_busy,
                    true,
                ));
        } else if r.status == Status::Available {
            actions = actions.push(square_action_button(
                ICON_ACTION_INSTALL,
                "Update",
                Message::AskAppUpdate,
                !busy && !queue_busy,
                true,
            ));
        }

        if !self.confirm_app_update
            && !self.confirm_app_reinstall
            && updater::can_reinstall_app(&self.updater)
        {
            actions = actions.push(square_action_button(
                ICON_ACTION_REINSTALL,
                "Reinstall",
                Message::AskAppReinstall,
                !busy && !queue_busy,
                false,
            ));
        }
        actions = actions.push(square_action_button(
            ICON_ACTION_UNINSTALL,
            "Uninstall",
            Message::OpenUninstallDialog,
            self.installation_info.can_uninstall && !busy && !queue_busy,
            false,
        ));

        let progress: Element<'_, Message> = match &r.status {
            Status::Downloading(progress) => custom_progress_bar(*progress),
            _ => Space::new().height(0).into(),
        };

        container(
            column![
                row![info, actions].spacing(12).align_y(Alignment::Center),
                progress,
            ]
            .spacing(8),
        )
        .style(t::card)
        .padding(14)
        .width(Length::Fill)
        .into()
    }

    fn tool_management_card(
        &self,
        comp: Component,
        r: updater::Row,
        busy: bool,
        queue_busy: bool,
    ) -> Element<'_, Message> {
        let installed = r.installed.clone().unwrap_or_else(|| "Not installed".to_string());
        let update_display_active = !r.missing
            && r.update_available
            && matches!(
                &r.status,
                Status::Available | Status::Downloading(_) | Status::Installing
            );
        let version = if update_display_active {
            format!("{installed} > {}", r.latest.clone().unwrap_or_default())
        } else {
            installed
        };
        let version_widget = text(version).size(12);
        let version_widget = if r.missing {
            version_widget.style(|_| iced::widget::text::Style { color: Some(t::danger()) })
        } else if update_display_active {
            version_widget.style(|_| iced::widget::text::Style { color: Some(t::warning()) })
        } else {
            match &r.status {
                Status::UpToDate => version_widget.style(|_| iced::widget::text::Style { color: Some(t::success()) }),
                Status::Failed(_) => version_widget.style(|_| iced::widget::text::Style { color: Some(t::danger()) }),
                Status::Downloading(_) | Status::Installing if !r.update_available => {
                    version_widget.style(|_| iced::widget::text::Style { color: Some(t::success()) })
                }
                _ => version_widget.style(t::muted_text),
            }
        };
        let size = tool_size_description(&r);
        let mut version_line = Row::new()
            .spacing(6)
            .align_y(Alignment::Center)
            .push(version_widget);
        if let Some(size) = size {
            version_line = version_line.push(
                text(format!("({size})")).size(11).style(t::muted_text),
            );
        }

        let status_text = match &r.status {
            Status::Unchecked => Some("Version not checked".to_string()),
            Status::Checking => Some("Checking…".to_string()),
            Status::Downloading(progress) => Some(format!("Downloading {:.0}%", *progress * 100.0)),
            Status::Installing => Some("Installing…".to_string()),
            Status::Removing => Some("Uninstalling…".to_string()),
            Status::Failed(error) => Some(error.clone()),
            Status::Unsupported(message) => Some(message.clone()),
            Status::UpToDate | Status::Available => None,
        };

        let mut info = column![text(comp.label()).size(13), version_line]
            .spacing(6)
            .width(Length::Fill);

        let mut tags: Vec<Element<'static, Message>> = Vec::new();
        if r.managed {
            tags.push(status_tag(
                "Managed",
                None,
                "A separate copy maintained by Musicerer.".to_string(),
                t::text_secondary(),
            ));
        }
        if r.external {
            tags.push(status_tag(
                "Path",
                None,
                "Found through system PATH; left untouched.".to_string(),
                t::text_secondary(),
            ));
        } else if !r.managed && r.origin.contains("Local copy") {
            tags.push(status_tag(
                "Local",
                None,
                "Found in a local tools folder; left untouched.".to_string(),
                t::text_secondary(),
            ));
        }
        if r.missing {
            let detail = if r.note.is_empty() {
                "This required tool is not installed.".to_string()
            } else {
                "This required tool is not fully installed.".to_string()
            };
            tags.push(status_tag("Missing", Some(ICON_ALERT_TRIANGLE), detail, t::danger()));
        } else if update_display_active {
            tags.push(status_tag(
                "Update",
                Some(ICON_ALERT_TRIANGLE),
                "A newer version is available.".to_string(),
                t::warning(),
            ));
        }

        if !tags.is_empty() {
            let footer = Row::with_children(tags).spacing(6).align_y(Alignment::Center);
            info = info.push(footer);
        }

        if let Some(status) = status_text {
            let status_widget = text(status).size(11);
            let status_widget = if matches!(&r.status, Status::Failed(_)) {
                status_widget.style(|_| iced::widget::text::Style { color: Some(t::danger()) })
            } else {
                status_widget.style(t::muted_text)
            };
            if let Status::Downloading(progress) = &r.status {
                info = info.push(
                    column![status_widget, custom_progress_bar(*progress)]
                        .spacing(3)
                        .width(Length::Fill),
                );
            } else {
                info = info.push(status_widget);
            }
        }

        let can_manage = !busy && !queue_busy;
        let mut actions = Column::new()
            .spacing(4)
            .width(Length::Fixed(36.0))
            .align_x(Alignment::End);
        if r.missing || r.external || !r.managed {
            let label = if r.missing { "Install" } else { "Install managed copy" };
            actions = actions.push(square_action_button(
                ICON_ACTION_INSTALL,
                label,
                Message::InstallManagedTool(comp),
                can_manage && updater::can_install_managed(&self.updater, comp),
                true,
            ));
        } else if r.status == Status::Available {
            actions = actions.push(square_action_button(
                ICON_ACTION_INSTALL,
                "Update",
                Message::InstallUpdate(comp),
                can_manage && r.can_update(),
                true,
            ));
        }
        if r.managed {
            if !r.missing && !r.external {
                actions = actions.push(square_action_button(
                    ICON_ACTION_REINSTALL,
                    "Reinstall",
                    Message::ReinstallManagedTool(comp),
                    can_manage && updater::can_reinstall_tool(&self.updater, comp),
                    false,
                ));
            }
            actions = actions.push(square_action_button(
                ICON_ACTION_UNINSTALL,
                "Uninstall",
                Message::UninstallManagedTool(comp),
                can_manage && !busy,
                false,
            ));
        }

        container(row![info, actions].spacing(12).align_y(Alignment::Center))
            .style(t::card)
            .padding(14)
            .width(Length::Fill)
            .into()
    }

    fn external_tools_card(&self, busy: bool, queue_busy: bool) -> Element<'_, Message> {
        let can_install = updater::can_install_external(&self.updater) && !busy && !queue_busy;
        container(
            column![
                text("External tools").size(14),
                text(
                    "Musicerer detected externally installed tools on your system. To avoid modifying your system-wide installations or interfering with your package manager, Musicerer will leave those installations untouched.\n\nYou can download separate copies specifically for Musicerer. These copies will be stored in its own application directory and managed independently."
                )
                .size(12)
                .style(t::muted_text),
                Space::new().height(4),
                button(text("Download managed copies").size(13))
                    .on_press_maybe(can_install.then_some(Message::InstallExternalTools))
                    .style(t::primary_button)
                    .padding([9, 14]),
            ]
            .spacing(10),
        )
        .style(t::card)
        .padding(14)
        .width(Length::Fill)
        .into()
    }

    fn installation_information_card(&self) -> Element<'_, Message> {
        let config = dirs::config_dir().map(|path| path.join("Musicerer"));
        let tools = crate::backend::tools::managed_dir();
        let cache = dirs::cache_dir().map(|path| path.join("Musicerer"));
        let update_cache = Some(std::env::temp_dir().join("Musicerer").join("updates"));
        let data = dirs::data_dir().map(|path| path.join("Musicerer"));
        let local_data = dirs::data_local_dir().map(|path| path.join("Musicerer"));
        let executable = self.installation_info.executable.clone();
        let executable_directory = executable
            .as_ref()
            .and_then(|path| path.parent().map(Path::to_path_buf));
        let download_directory = Some(PathBuf::from(self.output_dir.clone()));

        container(
            column![
                installation_info_entry(
                    "Application executable",
                    displayed_path(&executable),
                    executable_directory,
                ),
                installation_info_entry("Configuration directory", displayed_path(&config), config.clone()),
                installation_info_entry("Managed tools directory", displayed_path(&tools), tools.clone()),
                installation_info_entry("Cache directory", displayed_path(&cache), cache.clone()),
                installation_info_entry("Update staging directory", displayed_path(&update_cache), update_cache.clone()),
                installation_info_entry("User data directory", displayed_path(&data), data.clone()),
                installation_info_entry("Local user data directory", displayed_path(&local_data), local_data.clone()),
                installation_info_entry("Download directory", displayed_path(&download_directory), download_directory.clone()),
            ]
            .spacing(2),
        )
        .style(t::card)
        .padding(8)
        .width(Length::Fill)
        .into()
    }

    pub(super) fn view_uninstall_dialog(&self) -> Element<'_, Message> {
        let options = &self.uninstall_options;
        let busy = updater::is_busy(&self.updater) || self.downloading || self.is_loading();
        let output_path = self.output_dir.clone();

        let header = row![
            column![
                text("Uninstall Musicerer").size(19),
                text("Choose which application data to keep. Anything not selected is removed; downloaded media is always preserved.")
                    .size(12)
                    .style(t::muted_text),
            ]
            .spacing(4)
            .width(Length::Fill),
            dialog_close_button(Message::CloseUninstallDialog),
        ]
        .spacing(12)
        .align_y(Alignment::Center);

        let keep_options = column![
            checkbox(options.keep_settings)
                .label("Settings and preferences")
                .text_size(13)
                .size(18)
                .spacing(11)
                .width(Length::Fill)
                .style(t::dialog_checkbox)
                .on_toggle(Message::SetKeepSettings),
            checkbox(options.keep_tools)
                .label("Downloaded managed tools (yt-dlp, Deno, FFmpeg and FFprobe)")
                .text_size(13)
                .size(18)
                .spacing(11)
                .width(Length::Fill)
                .style(t::dialog_checkbox)
                .on_toggle(Message::SetKeepTools),
            checkbox(options.keep_cache)
                .label("Cache and temporary application data")
                .text_size(13)
                .size(18)
                .spacing(11)
                .width(Length::Fill)
                .style(t::dialog_checkbox)
                .on_toggle(Message::SetKeepCache),
            checkbox(options.keep_logs)
                .label("Application logs and diagnostic files")
                .text_size(13)
                .size(18)
                .spacing(11)
                .width(Length::Fill)
                .style(t::dialog_checkbox)
                .on_toggle(Message::SetKeepLogs),
            checkbox(options.keep_other_data)
                .label("Other Musicerer user data and custom files")
                .text_size(13)
                .size(18)
                .spacing(11)
                .width(Length::Fill)
                .style(t::dialog_checkbox)
                .on_toggle(Message::SetKeepOtherData),
        ]
        .spacing(13)
        .width(Length::Fill);

        let body = column![
            text("Keep application data").size(14),
            container(keep_options)
                .style(t::card)
                .padding(14)
                .width(Length::Fill),
            Space::new().height(3),
            container(column![
                text("Downloaded media is always kept").size(13),
                text(format!("Your downloads will remain in: {output_path}"))
                    .size(11)
                    .style(t::muted_text),
            ].spacing(6))
                .style(t::card)
                .padding([12, 14])
                .width(Length::Fill),
                text("Application files are always removed. Downloaded media and its output folder are preserved regardless of the selections above.")
                .size(11)
                .style(t::muted_text),
        ]
        .spacing(11)
        .width(Length::Fill);

        let footer = row![
            text("Nothing selected = remove optional application data.")
                .size(11)
                .style(t::muted_text),
            Space::new().width(Length::Fill),
            button(text("Cancel").size(13))
                .on_press(Message::CloseUninstallDialog)
                .style(t::secondary_button)
                .padding([9, 16]),
            button(text("Uninstall").size(13))
                .on_press_maybe((!busy).then_some(Message::ConfirmUninstall))
                .style(t::primary_button)
                .padding([9, 16]),
        ]
        .spacing(9)
        .align_y(Alignment::Center);

        let card = container(
            column![
                header,
                container(Space::new().height(1))
                    .width(Length::Fill)
                    .style(|_| iced::widget::container::Style {
                        background: Some(iced::Background::Color(t::border_subtle())),
                        ..Default::default()
                    }),
                scrollable(body).height(Length::Fill),
                container(Space::new().height(1))
                    .width(Length::Fill)
                    .style(|_| iced::widget::container::Style {
                        background: Some(iced::Background::Color(t::border_subtle())),
                        ..Default::default()
                    }),
                footer,
            ]
            .spacing(12)
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .style(t::card_elevated)
        .padding(20)
        .width(Length::Fill)
        .height(Length::Fill)
        .max_width(640.0)
        .max_height(700.0);

        container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding([12, 14])
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(|_| iced::widget::container::Style {
                background: Some(iced::Background::Color(Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.78,
                })),
                ..Default::default()
            })
            .into()
    }

    pub(super) fn view_tool_info_dialog(&self) -> Element<'_, Message> {
        let managed_path = crate::backend::tools::managed_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "Unavailable on this system".to_string());

        let header = row![
            text("How tool management works").size(18),
            Space::new().width(Length::Fill),
            dialog_close_button(Message::ToggleToolInfoDialog),
        ]
        .align_y(Alignment::Center);

        let body = column![
            help_section_heading("What the tools do"),
            help_tool_row("yt-dlp", "Downloads audio from supported sites."),
            help_tool_row("Deno", "Helps yt-dlp with some site checks."),
            help_tool_row("FFmpeg", "Processes audio."),
            help_tool_row("FFprobe", "Reads audio information."),
            help_section_heading("Where the copies come from"),
            row![
                status_tag("Managed", None, "A separate copy maintained by Musicerer.".to_string(), t::text_secondary()),
                status_tag("Local", None, "Found in a local tools folder.".to_string(), t::text_secondary()),
                status_tag("Path", None, "Found through your system PATH.".to_string(), t::text_secondary()),
            ]
            .spacing(6),
            text("Musicerer checks Managed first, then Local, then Path. It uses the first copy it finds.").size(12),
            help_section_heading("Your system tools stay untouched"),
            text("Installing, updating or removing a Managed copy only changes Musicerer's own copy. Other copies stay as they are.").size(12),
            help_section_heading("Sizes"),
            text("For Path tools, the sizes show the existing copy + the separate managed copy. For managed updates, + or − shows the space added or freed. Sizes stay fixed while downloading.").size(12),
            help_section_heading("Managed tools folder"),
            container(text(managed_path).size(11).style(t::muted_text))
                .style(t::card)
                .padding(9)
                .width(Length::Fill),
        ]
        .spacing(8)
        .width(Length::Fill);
        let card = container(
            column![
                header,
                container(Space::new().height(1))
                    .width(Length::Fill)
                    .style(|_| iced::widget::container::Style {
                        background: Some(iced::Background::Color(t::border_subtle())),
                        ..Default::default()
                    }),
                scrollable(body).height(Length::Fill),
            ]
            .spacing(12)
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .style(t::card_elevated)
        .padding(20)
        .width(Length::Fill)
        .height(Length::Fill)
        .max_width(580.0)
        .max_height(620.0);

        container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding([12, 14])
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(|_| iced::widget::container::Style {
                background: Some(iced::Background::Color(Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.78,
                })),
                ..Default::default()
            })
            .into()
    }

}
