use iced::{
    widget::{button, column, container, pick_list, row, text},
    Element, Length,
};

use crate::app::AppMessage;
use crate::i18n::t;
use crate::settings::{AppTheme, Settings};

/// Wrapper for language display in pick_list
#[derive(Debug, Clone, PartialEq, Eq)]
struct LangOption {
    code: String,
    display: String,
}

impl std::fmt::Display for LangOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display)
    }
}

pub fn view<'a>(settings: &'a Settings) -> Element<'a, AppMessage> {
    // ---- Application card ----
    let theme_pick = pick_list(
        vec![AppTheme::Light, AppTheme::Dark],
        Some(settings.theme),
        AppMessage::SettingsChangeTheme,
    );

    let lang_display_names = crate::i18n::language_display_names();
    let lang_options: Vec<LangOption> = lang_display_names
        .iter()
        .map(|(code, display)| LangOption {
            code: code.clone(),
            display: display.clone(),
        })
        .collect();
    let current_lang = lang_options
        .iter()
        .find(|o| o.code == settings.language)
        .cloned();
    let lang_pick = pick_list(
        lang_options,
        current_lang,
        |opt: LangOption| AppMessage::SettingsChangeLanguage(opt.code),
    );

    let save_device_toggle = button(
        text(if settings.save_last_device { "ON" } else { "OFF" }).size(13),
    )
    .padding([4, 12])
    .on_press(AppMessage::SettingsToggleSaveDevice);

    let app_card = container(
        column![
            text(t!("settings.application").to_string()).size(16),
            iced::widget::Space::new().height(10.0),
            row![
                text(t!("settings.theme").to_string()).size(14),
                iced::widget::space::horizontal(),
                theme_pick,
            ].align_y(iced::Alignment::Center),
            row![
                text(t!("settings.language").to_string()).size(14),
                iced::widget::space::horizontal(),
                lang_pick,
            ].align_y(iced::Alignment::Center),
            row![
                text(t!("settings.save_device").to_string()).size(14),
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
                text(t!("settings.about").to_string()).size(16),
                iced::widget::Space::new().width(10.0),
                text("ⓘ").size(20),
            ].align_y(iced::Alignment::Center),
            iced::widget::Space::new().height(10.0),
            text(format!("{} v{}", t!("app.title"), env!("CARGO_PKG_VERSION"))).size(14),
            row![
                text(t!("settings.repository").to_string()).size(14),
                button(text("GitHub").size(13))
                    .padding([3, 8])
                    .on_press(AppMessage::SettingsOpenRepo),
            ].spacing(6).align_y(iced::Alignment::Center),
            text("https://github.com/rssdev10/skyrc-mc-rs").size(12),
        ]
        .spacing(6),
    )
    .padding(16)
    .style(container::bordered_box)
    .width(Length::Fill);

    container(
        column![
            text(t!("settings.title").to_string()).size(22),
            iced::widget::Space::new().height(16.0),
            app_card,
            iced::widget::Space::new().height(16.0),
            about_card,
            iced::widget::Space::new().height(12.0),
            text(t!("settings.auto_saved").to_string()).size(12),
            iced::widget::Space::new().height(Length::Fill),
            button(text(t!("btn.close").to_string()).size(14)).on_press(AppMessage::SettingsClose),
        ]
        .spacing(0)
        .padding(20),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}
