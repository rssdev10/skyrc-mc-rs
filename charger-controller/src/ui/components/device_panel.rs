use iced::{
    widget::{button, container, pick_list, row, text},
    Element, Length,
};

use crate::app::{AppMessage, ConnectionStatus};
use mc5000_protocol::{Device, DeviceManager};

pub fn view<'a>(
    device_manager: &'a DeviceManager,
    connected_device: &'a Option<Device>,
    connection_status: &'a ConnectionStatus,
    selected_device: &'a Option<String>,
    scanning: bool,
) -> Element<'a, AppMessage> {
    let status_text: Element<AppMessage> = match connection_status {
        ConnectionStatus::Disconnected => text("⊘ Disconnected")
            .size(13)
            .color(iced::Color::from_rgb(0.8, 0.2, 0.2))
            .into(),
        ConnectionStatus::Connecting => text("⟳ Connecting...")
            .size(13)
            .color(iced::Color::from_rgb(0.2, 0.6, 1.0))
            .into(),
        ConnectionStatus::Connected => text("● Connected")
            .size(13)
            .color(iced::Color::from_rgb(0.2, 0.8, 0.2))
            .into(),
        ConnectionStatus::Error(msg) => text(format!("⚠ {}", msg))
            .size(13)
            .color(iced::Color::from_rgb(0.8, 0.2, 0.2))
            .into(),
    };

    let device_controls: Element<AppMessage> = if connected_device.is_none() {
        let available_devices = device_manager.get_available_devices().to_vec();

        let device_picker: Element<AppMessage> = if scanning {
            text("⟳ Scanning...").size(13).into()
        } else if available_devices.is_empty() {
            text("No devices found").size(13).into()
        } else {
            pick_list(
                available_devices,
                selected_device.clone(),
                AppMessage::DeviceSelected,
            )
            .placeholder("Select device...")
            .into()
        };

        let refresh_button = button(text("Refresh").size(13))
            .on_press_maybe(if scanning { None } else { Some(AppMessage::RefreshDevices) })
            .padding([4, 8]);

        let connect_button = button(text("Connect").size(13))
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
        // For BT devices: "MC5000 BT: #Charger XXXX (ID:...)" → show just "#Charger XXXX"
        // let display_name = if name.starts_with("MC5000 BT: ") {
        //     name.strip_prefix("MC5000 BT: ")
        //         .and_then(|s| s.find(" (ID:").map(|pos| s[..pos].to_string()))
        //         .unwrap_or_else(|| name.clone())
        // } else {
        //     name.clone()
        // };
        let display_name = name.clone();  // Show full name for now, can simplify if needed

        row![
            text(format!("Connected to: {}", display_name)).size(13),
            button(text("Disconnect").size(13))
                .on_press(AppMessage::DisconnectDevice)
                .style(button::danger)
                .padding([4, 8]),
        ]
        .spacing(8)
        .align_y(iced::Center)
        .into()
    };

    container(
        row![
            device_controls,
            iced::widget::space::horizontal(),
            status_text,
        ]
        .spacing(10)
        .align_y(iced::Center)
    )
    .padding([8, 15])
    .style(container::bordered_box)
    .width(Length::Fill)
    .into()
}