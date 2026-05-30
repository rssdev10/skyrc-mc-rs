pub mod message;
pub mod components;

use iced::{
    widget::{button, column, container, row, text, tooltip},
    Element, Length,
};
use std::time::Duration;

use crate::app::{AppMessage, ConnectionStatus};
use crate::i18n::t;
use mc5000_protocol::{Device, DeviceManager};
use crate::slot::{Slot, SlotId, TaskConfig};
use crate::data::DataLogger;

use components::{device_panel, slot_panel, graph_panel, data_panel};

#[allow(clippy::too_many_arguments)]
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
    profile_store: &'a crate::profiles::ProfileStore,
) -> Element<'a, AppMessage> {
    // If config dialog is active, show it instead of the main view
    if let Some(state) = config_dialog_state {
        return components::config_dialog::view_config_dialog(state, profile_store);
    }

    // Device panel on top (connection controls + status)
    let device_section = device_panel::view(
        device_manager,
        connected_device,
        connection_status,
        selected_device,
        scanning,
    );

    // Slots section with Auto/Smart/Stop buttons in the header row
    let slots_section = create_slots_section(slots, slot_configs, configuring_slot, connection_status, selected_slot);

    // Data panel (stats + export) on the left, graph fills the rest
    let data_section = data_panel::view(data_logger, show_detailed_stats);
    let graph_section = graph_panel::view(data_logger, slots, selected_slot);

    let bottom_section = row![
        container(data_section).width(Length::Fixed(280.0)).height(Length::Fill),
        graph_section,
    ]
    .spacing(10);

    let content = column![
        device_section,
        slots_section,
        bottom_section,
    ]
    .spacing(10)
    .padding(10);

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
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

    // Auto/SmartCharge/Stop All buttons aligned right
    let auto_button = if is_connected {
        button(text(t!("btn.auto").to_string()).size(14))
            .on_press(AppMessage::SimpleAutoCharge)
            .padding([4, 10])
            .style(button::primary)
    } else {
        button(text(t!("btn.auto").to_string()).size(14)).padding([4, 10])
    };
    let auto_tooltip = tooltip(
        auto_button,
        container(text(t!("tooltip.auto").to_string()).size(12)).padding(5).style(container::rounded_box),
        tooltip::Position::Bottom,
    )
    .delay(Duration::from_millis(500));

    let smart_button = if is_connected {
        button(text(t!("btn.smart_charge").to_string()).size(14))
            .on_press(AppMessage::SmartChargeAll)
            .padding([4, 10])
            .style(button::secondary)
    } else {
        button(text(t!("btn.smart_charge").to_string()).size(14)).padding([4, 10])
    };
    let smart_tooltip = tooltip(
        smart_button,
        container(text(t!("tooltip.smart_charge").to_string()).size(12)).padding(5).style(container::rounded_box),
        tooltip::Position::Bottom,
    )
    .delay(Duration::from_millis(500));

    let stop_all_button = if is_connected {
        button(text(t!("btn.stop_all").to_string()).size(14))
            .on_press(AppMessage::StopAllSlots)
            .padding([4, 10])
            .style(button::danger)
    } else {
        button(text(t!("btn.stop_all").to_string()).size(14)).padding([4, 10])
    };
    let stop_all_tooltip = tooltip(
        stop_all_button,
        container(text(t!("tooltip.stop_all").to_string()).size(12)).padding(5).style(container::rounded_box),
        tooltip::Position::Bottom,
    )
    .delay(Duration::from_millis(500));

    let header_row = row![
        text(t!("label.charging_slots").to_string()).size(18),
        iced::widget::space::horizontal(),
        auto_tooltip,
        smart_tooltip,
        stop_all_tooltip,
    ]
    .spacing(5)
    .align_y(iced::Center);

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
        header_row,
        row(slot_views).spacing(10),
    ]
    .spacing(5)
    .into()
}