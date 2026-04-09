use iced::{
    widget::{button, column, container, pick_list, row, text},
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
    let device_selection = if connected_device.is_none() {
        let available_devices = device_manager.get_available_devices().to_vec();
        
        let device_picker: Element<AppMessage> = if scanning {
            text("⟳ Scanning for devices...").into()
        } else if available_devices.is_empty() {
            text("No devices found").into()
        } else {
            pick_list(
                available_devices,
                selected_device.clone(),
                AppMessage::DeviceSelected,
            )
            .placeholder("Select device...")
            .into()
        };

        let refresh_button = button("Refresh")
            .on_press_maybe(if scanning { None } else { Some(AppMessage::RefreshDevices) })
            .padding(6);

        let connect_button = button("Connect")
            .on_press_maybe(
                if selected_device.is_some() && *connection_status != ConnectionStatus::Connecting {
                    Some(AppMessage::ConnectDevice)
                } else {
                    None
                }
            );

        row![device_picker, connect_button, refresh_button]
            .spacing(10)
            .align_items(iced::Alignment::Center)
    } else {
        row![
            text(format!("Connected to: {}", 
                connected_device.as_ref().unwrap().name
            )),
            button("Disconnect")
                .on_press(AppMessage::DisconnectDevice)
                .style(iced::theme::Button::Destructive),
        ]
        .spacing(10)
        .align_items(iced::Alignment::Center)
    };

    container(
        column![
            text("Device Connection").size(16),
            device_selection,
        ]
        .spacing(10)
    )
    .padding(15)
    .style(iced::theme::Container::Box)
    .width(Length::Fill)
    .into()
}