//! UI for application and dependency installation management.

use super::{App, Message};
use crate::backend::updater::{self, Component, Status};
use crate::theme as t;
use crate::ui::{
    custom_progress_bar,
    icons::{ICON_ACTION_CANCEL, ICON_ACTION_CONFIRM, ICON_ACTION_INSTALL, ICON_ACTION_REINSTALL, ICON_ACTION_UNINSTALL},
};

use iced::widget::{
    button, checkbox, column, container, row, scrollable, svg, tooltip, Column, Space, text,
};
use iced::{Alignment, Color, Element, Length};
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

        let header = row![
            text("Installation").size(20),
            Space::new().width(Length::Fill),
            button(text("Check for updates").size(13))
                .on_press_maybe((!busy).then_some(Message::CheckUpdates))
                .style(t::update_check_button)
                .padding([8, 14]),
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
            Status::Available => "Update available".to_string(),
        };
        let status_color = match &r.status {
            Status::Available | Status::Failed(_) => Some(t::danger()),
            Status::UpToDate => Some(t::success()),
            _ => None,
        };
        let version_widget = text(version).size(12);
        let version_widget = if r.status == Status::Available {
            version_widget.style(|_| iced::widget::text::Style { color: Some(t::danger()) })
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

        let actions_width = if self.confirm_app_update
            || self.confirm_app_reinstall
            || r.status == Status::Available
        {
            142.0
        } else {
            36.0
        };
        let mut actions = Column::new()
            .spacing(5)
            .width(Length::Fixed(actions_width))
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
                .push(
                    button(text("Restart & update").size(12))
                        .on_press_maybe((!busy && !queue_busy).then_some(Message::InstallUpdate(Component::App)))
                        .style(t::primary_button)
                        .padding([7, 10])
                        .width(Length::Fill),
                );
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
            actions = actions.push(
                button(text("Update").size(12))
                    .on_press_maybe((!busy && !queue_busy).then_some(Message::AskAppUpdate))
                    .style(t::primary_button)
                    .padding([7, 10])
                    .width(Length::Fill),
            );
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
        let version = if r.status == Status::Available {
            format!("{installed}  →  {}", r.latest.clone().unwrap_or_default())
        } else {
            installed
        };
        let status_text = match &r.status {
            Status::Unchecked => "Version not checked".to_string(),
            Status::Checking => "Checking…".to_string(),
            Status::Downloading(progress) => format!("Downloading {:.0}%", *progress * 100.0),
            Status::Installing => "Installing…".to_string(),
            Status::Removing => "Uninstalling…".to_string(),
            Status::Failed(error) => error.clone(),
            Status::Unsupported(message) => message.clone(),
            Status::UpToDate if r.missing => "Missing · installation required".to_string(),
            Status::Available if r.missing => "Missing · installation required".to_string(),
            Status::UpToDate => "Up to date".to_string(),
            Status::Available => "Update available".to_string(),
        };
        let version_widget = text(version).size(12);
        let version_widget = match &r.status {
            Status::UpToDate => version_widget.style(|_| iced::widget::text::Style { color: Some(t::success()) }),
            Status::Available | Status::Failed(_) => version_widget.style(|_| iced::widget::text::Style { color: Some(t::danger()) }),
            _ => version_widget.style(t::muted_text),
        };
        let status_widget = text(status_text).size(11);
        let status_widget = if let Status::Failed(_) = &r.status {
            status_widget.style(|_| iced::widget::text::Style { color: Some(t::danger()) })
        } else {
            status_widget.style(t::muted_text)
        };
        let mut info = column![text(comp.label()).size(13), version_widget, status_widget, text(r.origin.clone()).size(11).style(t::muted_text)]
            .spacing(3)
            .width(Length::Fill);
        if !r.note.is_empty() && r.note != r.origin {
            info = info.push(text(r.note.clone()).size(11).style(t::muted_text));
        }
        if let Status::Downloading(progress) = &r.status {
            info = info.push(Space::new().height(3)).push(custom_progress_bar(*progress));
        }

        let can_manage = !busy && !queue_busy;
        let actions_width = if r.status == Status::Available && r.managed && !r.external && !r.missing { 142.0 } else { 36.0 };
        let mut actions = Column::new()
            .spacing(5)
            .width(Length::Fixed(actions_width))
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
            actions = actions.push(
                button(text("Update").size(12).width(Length::Fill).align_x(Alignment::Center))
                    .on_press_maybe((can_manage && r.can_update()).then_some(Message::InstallUpdate(comp)))
                    .style(t::primary_button)
                    .padding([7, 10]),
            );
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
        let output_notice = if options.keep_output_directory {
            format!("Downloaded media will be kept at: {output_path}")
        } else {
            format!("The entire folder will be deleted, including files Musicerer did not create:\n{output_path}")
        };

        let header = row![
            column![
                text("Uninstall Musicerer").size(19),
                text("Choose which data to keep. Anything not selected will be removed.")
                    .size(12)
                    .style(t::muted_text),
            ]
            .spacing(4)
            .width(Length::Fill),
            button(text("Close").size(12))
                .on_press(Message::CloseUninstallDialog)
                .style(t::secondary_button)
                .padding([3, 10]),
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

        let output_options = column![
            checkbox(options.keep_output_directory)
                .label("Keep downloaded media and the configured output folder")
                .text_size(13)
                .size(18)
                .spacing(11)
                .width(Length::Fill)
                .style(t::dialog_checkbox)
                .on_toggle(Message::SetKeepOutputDirectory),
            text(output_notice).size(11).style(t::muted_text),
        ]
        .spacing(8)
        .width(Length::Fill);

        let body = column![
            text("Keep application data").size(14),
            container(keep_options)
                .style(t::card)
                .padding(14)
                .width(Length::Fill),
            Space::new().height(3),
            text("Downloaded media").size(14),
            container(output_options)
                .style(t::card)
                .padding(14)
                .width(Length::Fill),
            text("Application files are always removed. Select only the data you want to preserve.")
                .size(11)
                .style(t::muted_text),
        ]
        .spacing(11)
        .width(Length::Fill);

        let footer = row![
            text("Nothing selected = remove all Musicerer data.")
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

}
