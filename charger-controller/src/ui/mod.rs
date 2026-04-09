pub mod message;
pub mod components;

use iced::{
    widget::{column, container, row, text},
    Element, Length,
};

use crate::app::{AppMessage, ConnectionStatus};
use mc5000_protocol::{Device, DeviceManager};
use crate::slot::{Slot, SlotId, TaskConfig};
use crate::data::DataLogger;

use components::{device_panel, slot_panel, data_panel, graph_panel};

pub fn main_view<'a>(
    device_manager: &'a DeviceManager,
    connected_device: &'a Option<Device>,
    slots: &'a [Slot; 4],
    data_logger: &'a DataLogger,
    connection_status: &'a ConnectionStatus,
    selected_device: &'a Option<String>,
    slot_configs: &'a [Option<TaskConfig>; 4],
    configuring_slot: &'a Option<SlotId>,
    scanning: bool,
    selected_slot: Option<usize>,
    config_dialog_state: &'a Option<components::config_dialog::ConfigDialogState>,
    show_detailed_stats: bool,
) -> Element<'a, AppMessage> {
    // If config dialog is active, show it instead of the main view
    if let Some(state) = config_dialog_state {
        return components::config_dialog::view_config_dialog(state);
    }

    let header = create_header(connection_status);
    
    let device_section = device_panel::view(
        device_manager,
        connected_device,
        connection_status,
        selected_device,
        scanning,
    );

    let slots_section = create_slots_section(slots, slot_configs, configuring_slot, connection_status, selected_slot);
    
    let data_section = row![
        data_panel::view(data_logger, show_detailed_stats),
        graph_panel::view(data_logger, slots, selected_slot),
    ]
    .spacing(20);

    let content = column![
        header,
        device_section,
        slots_section,
        data_section,
    ]
    .spacing(20)
    .padding(20);

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn create_header(connection_status: &ConnectionStatus) -> Element<AppMessage> {
    use iced::widget::button;
    
    let title = text("Multi-Slot Charger Controller")
        .size(24);

    let status_text = match connection_status {
        ConnectionStatus::Disconnected => text("Disconnected").style(iced::Color::from_rgb(0.8, 0.2, 0.2)),
        ConnectionStatus::Connecting => text("Connecting...").style(iced::Color::from_rgb(0.2, 0.6, 1.0)),
        ConnectionStatus::Connected => text("Connected").style(iced::Color::from_rgb(0.2, 0.8, 0.2)),
        ConnectionStatus::Error(msg) => text(format!("Error: {}", msg)).style(iced::Color::from_rgb(0.8, 0.2, 0.2)),
    };
    
    // Simple Auto button (detect Li-Ion/NiMH, charge at 500mA)
    let auto_button = if matches!(connection_status, ConnectionStatus::Connected) {
        button(
            text("Auto")
                .size(16)
        )
        .on_press(AppMessage::SimpleAutoCharge)
        .padding(10)
        .style(iced::theme::Button::Primary)
    } else {
        button(
            text("Auto")
                .size(16)
        )
        .padding(10)
    };
    
    // SmartCharge button (detect chemistry, measure resistance, optimize current)
    let smart_button = if matches!(connection_status, ConnectionStatus::Connected) {
        button(
            text("SmartCharge")
                .size(16)
        )
        .on_press(AppMessage::SmartChargeAll)
        .padding(10)
        .style(iced::theme::Button::Secondary)
    } else {
        button(
            text("SmartCharge")
                .size(16)
        )
        .padding(10)
    };

    // Stop All button
    let stop_all_button = if matches!(connection_status, ConnectionStatus::Connected) {
        button(
            text("Stop All")
                .size(16)
        )
        .on_press(AppMessage::StopAllSlots)
        .padding(10)
        .style(iced::theme::Button::Destructive)
    } else {
        button(
            text("Stop All")
                .size(16)
        )
        .padding(10)
    };

    row![
        title,
        iced::widget::horizontal_space(),
        auto_button,
        iced::widget::Space::with_width(Length::Fixed(10.0)),
        smart_button,
        iced::widget::Space::with_width(Length::Fixed(10.0)),
        stop_all_button,
        iced::widget::horizontal_space().width(Length::Fixed(20.0)),
        status_text,
    ]
    .align_items(iced::Alignment::Center)
    .into()
}

fn create_slots_section<'a>(
    slots: &'a [Slot; 4],
    slot_configs: &'a [Option<TaskConfig>; 4],
    configuring_slot: &'a Option<SlotId>,
    connection_status: &'a ConnectionStatus,
    selected_slot: Option<usize>,
) -> Element<'a, AppMessage> {
    let is_connected = matches!(connection_status, ConnectionStatus::Connected);
    
    let slot_views: Vec<Element<AppMessage>> = slots
        .iter()
        .enumerate()
        .map(|(idx, slot)| {
            let config = slot_configs.get(idx).and_then(|c| c.as_ref());
            let is_configuring = configuring_slot.map(|id| id.0 == idx).unwrap_or(false);
            let is_selected = selected_slot == Some(idx);
            slot_panel::view(slot, config, is_configuring, is_connected, is_selected, idx)
        })
        .collect();

    column![
        text("Charging Slots").size(18),
        row(slot_views).spacing(10),
    ]
    .spacing(10)
    .into()
}