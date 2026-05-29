use iced::{
    widget::{button, column, container, pick_list, row, text},
    Element, Length,
};

use crate::app::AppMessage;
use crate::settings::{AppTheme, Settings};

pub fn view<'a>(settings: &'a Settings) -> Element<'a, AppMessage> {
    // ---- Application card ----
    let theme_pick = pick_list(
        vec![AppTheme::Dark],
        Some(settings.theme),
        AppMessage::SettingsChangeTheme,
    );

    let save_device_toggle = button(
        text(if settings.save_last_device { "ON" } else { "OFF" }).size(12),
    )
    .padding([4, 12])
    .on_press(AppMessage::SettingsToggleSaveDevice);

    let app_card = container(
        column![
            text("Application").size(15),
            iced::widget::Space::new().height(10.0),
            row![
                text("Theme").size(13),
                iced::widget::space::horizontal(),
                theme_pick,
            ].align_y(iced::Alignment::Center),
            row![
                text("Save last connected device").size(13),
                iced::widget::space::horizontal(),
                save_device_toggle,
            ].align_y(iced::Alignment::Center),
        ]
        .spacing(8),
    )
    .padding(16)
    .style(container::bordered_box)
    .width(Length::Fill);

    // ---- About card ----
    let about_card = container(
        column![
            row![
                text("About").size(15),
                iced::widget::Space::new().width(10.0),
                text("ⓘ").size(20),
            ].align_y(iced::Alignment::Center),
            iced::widget::Space::new().height(10.0),
            text(format!("MC5000 Charger Controller v{}", env!("CARGO_PKG_VERSION"))).size(13),
            row![
                text("Repository:").size(13),
                button(text("GitHub").size(12))
                    .padding([3, 8])
                    .on_press(AppMessage::SettingsOpenRepo),
            ].spacing(6).align_y(iced::Alignment::Center),
            text("https://github.com/rssdev10/skyrc-mc-rs").size(11),
        ]
        .spacing(6),
    )
    .padding(16)
    .style(container::bordered_box)
    .width(Length::Fill);

    container(
        column![
            text("Settings").size(22),
            iced::widget::Space::new().height(16.0),
            app_card,
            iced::widget::Space::new().height(16.0),
            about_card,
            iced::widget::Space::new().height(12.0),
            text("Settings are saved automatically").size(11),
            iced::widget::Space::new().height(Length::Fill),
            button(text("Close")).on_press(AppMessage::SettingsClose),
        ]
        .spacing(0)
        .padding(20),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}
