use iced::{
    widget::{button, container, pick_list, row, text},
    Element, Length,
};

use crate::app::{AppMessage, ConnectionStatus};
use crate::i18n::t;
use mc5000_protocol::{Device, DeviceManager};

pub fn view<'a>(
    device_manager: &'a DeviceManager,
    connected_device: &'a Option<Device>,
    connection_status: &'a ConnectionStatus,
    selected_device: &'a Option<String>,
    scanning: bool,
) -> Element<'a, AppMessage> {
    let status_text: Element<AppMessage> = match connection_status {
        ConnectionStatus::Disconnected => text(t!("label.disconnected").to_string())
            .size(14)
            .color(iced::Color::from_rgb(0.8, 0.2, 0.2))
            .into(),
        ConnectionStatus::Connecting => text(t!("label.connecting").to_string())
            .size(14)
            .color(iced::Color::from_rgb(0.2, 0.6, 1.0))
            .into(),
        ConnectionStatus::Connected => text(t!("label.connected").to_string())
            .size(14)
            .color(iced::Color::from_rgb(0.2, 0.8, 0.2))
            .into(),
        ConnectionStatus::Error(msg) => text(format!("⚠ {}", msg))
            .size(14)
            .color(iced::Color::from_rgb(0.8, 0.2, 0.2))
            .into(),
    };

    let device_controls: Element<AppMessage> = if connected_device.is_none() {
        let available_devices = device_manager.get_available_devices().to_vec();

        let device_picker: Element<AppMessage> = if scanning {
            text(t!("label.scanning").to_string()).size(14).into()
        } else if available_devices.is_empty() {
            text(t!("label.no_devices").to_string()).size(14).into()
        } else {
            pick_list(
                available_devices,
                selected_device.clone(),
                AppMessage::DeviceSelected,
            )
            .placeholder("Select device...")
            .into()
        };

        let refresh_button = button(text(t!("btn.refresh").to_string()).size(14))
            .on_press_maybe(if scanning { None } else { Some(AppMessage::RefreshDevices) })
            .padding([4, 8]);

        let connect_button = button(text(t!("btn.connect").to_string()).size(14))
            .on_press_maybe(
                if selected_device.is_some() && *connection_status != ConnectionStatus::Connecting {
                    Some(AppMessage::ConnectDevice)
                } else {
                    None
                }
            )
            .padding([4, 8]);

        row![device_picker, connect_button, refresh_button]
            .spacing(8)
            .align_y(iced::Center)
            .into()
    } else {
        let name = &connected_device.as_ref().unwrap().name;
        let display_name = name.clone();

        row![
            text(format!("{} {}", t!("label.connected_to"), display_name)).size(14),
            button(text(t!("btn.disconnect").to_string()).size(14))
                .on_press(AppMessage::DisconnectDevice)
                .style(button::danger)
                .padding([4, 8]),
        ]
        .spacing(8)
        .align_y(iced::Center)
        .into()
    };

    let settings_button = button(text("⚙").size(16))
        .on_press(AppMessage::SettingsOpen)
        .padding([4, 8]);

    container(
        row![
            device_controls,
            iced::widget::space::horizontal(),
            status_text,
            settings_button,
        ]
        .spacing(10)
        .align_y(iced::Center)
    )
    .padding([8, 15])
    .style(container::bordered_box)
    .width(Length::Fill)
    .into()
}