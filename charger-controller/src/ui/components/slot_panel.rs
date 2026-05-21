use iced::{
    widget::{button, column, container, row, text, progress_bar, pick_list, text_input, mouse_area},
    Element, Length, Border,
};

use crate::app::AppMessage;
use crate::slot::{Slot, SlotState, TaskConfig, TaskType, BatteryChemistry};

pub fn view<'a>(slot: &'a Slot, config: Option<&'a TaskConfig>, is_configuring: bool, is_connected: bool, is_selected: bool, slot_index: usize) -> Element<'a, AppMessage> {
    if is_configuring {
        return config_dialog_view(slot, config);
    }

    let slot_header = row![
        text(format!("Slot {}", slot.id.0 + 1)).size(16),
        iced::widget::space::horizontal(),
        status_badge(&slot.state),
    ]
    .align_y(iced::Center);

    let measurements = column![
        row![
            column![
                text("Voltage").size(12),
                text(format!("{:.3}V", slot.current_voltage)).size(14),
            ]
            .width(Length::FillPortion(1))
            .align_x(iced::Center),
            column![
                text("Current").size(12),
                text(format!("{}mA", slot.current_current as u16)).size(14),
            ]
            .width(Length::FillPortion(1))
            .align_x(iced::Center),
        ]
        .spacing(10),
        row![
            column![
                text("Power").size(12),
                text(format!("{:.1}W", slot.power_w())).size(14),
            ]
            .width(Length::FillPortion(1))
            .align_x(iced::Center),
            column![
                text("Capacity").size(12),
                text(format!("{}mAh", slot.capacity_mah)).size(14),
            ]
            .width(Length::FillPortion(1))
            .align_x(iced::Center),
        ]
        .spacing(10),
        row![
            column![
                text("Resistance").size(12),
                text(format!("{}mΩ", slot.resistance_milliohm)).size(14),
            ]
            .width(Length::FillPortion(1))
            .align_x(iced::Center),
            column![
                text("Time").size(12),
                text(format!("{:02}:{:02}", slot.elapsed_seconds / 60, slot.elapsed_seconds % 60)).size(14),
            ]
            .width(Length::FillPortion(1))
            .align_x(iced::Center),
        ]
        .spacing(10),
    ]
    .spacing(8);

    // Bottom row: progress bar + stop button (or centered configure button)
    let bottom_row: Element<AppMessage> = if slot.is_active() || slot.state == SlotState::Paused {
        let progress_value = slot.get_progress_percentage() / 100.0;
        let progress_section = column![
            row![
                text("Progress").size(12),
                iced::widget::space::horizontal(),
                text(format!("{:.1}%", slot.get_progress_percentage())).size(12),
            ],
            container(progress_bar(0.0..=1.0, progress_value))
                .height(Length::Fixed(8.0)),
        ]
        .spacing(2)
        .width(Length::Fill);

        let stop_button = button(text("Stop").size(13))
            .on_press(AppMessage::StopTask(slot.id))
            .style(button::danger)
            .padding([8, 12]);

        row![
            progress_section,
            iced::widget::Space::new().width(Length::Fixed(5.0)),
            stop_button,
        ]
        .align_y(iced::Center)
        .into()
    } else if slot.state == SlotState::Idle {
        // Centered configure button with 20px padding on each side
        let mut btn = button(text("Configure & Start").size(13))
            .style(button::primary)
            .padding([8, 12]);

        if is_connected {
            let voltage = slot.current_voltage;
            btn = btn.on_press(AppMessage::ShowConfigDialog(slot.id, voltage));
        }

        container(btn)
            .width(Length::Fill)
            .align_x(iced::Center)
            .padding([0, 10])
            .into()
    } else {
        // Completed or error - reset button centered
        let btn = button(text("Reset").size(13))
            .on_press(AppMessage::StopTask(slot.id))
            .style(button::secondary)
            .padding([8, 12]);

        container(btn)
            .width(Length::Fill)
            .align_x(iced::Center)
            .padding([0, 20])
            .into()
    };

    // Fixed-height content column
    let content = column![
        slot_header,
        measurements,
        bottom_row,
    ]
    .spacing(8)
    .padding(10)
    .height(Length::Fixed(220.0));

    // Determine slot color for selection border
    let slot_color = match slot_index {
        0 => iced::Color::from_rgb(1.0, 0.3, 0.3),
        1 => iced::Color::from_rgb(0.3, 1.0, 0.3),
        2 => iced::Color::from_rgb(0.3, 0.3, 1.0),
        3 => iced::Color::from_rgb(1.0, 1.0, 0.3),
        _ => iced::Color::WHITE,
    };

    let border_width = if is_selected { 3.0 } else { 1.0 };
    let border_color = if is_selected { slot_color } else { iced::Color::from_rgb(0.3, 0.3, 0.3) };

    mouse_area(
        container(content)
            .style(move |_theme: &iced::Theme| {
                container::Style {
                    border: Border {
                        color: border_color,
                        width: border_width,
                        radius: 5.0.into(),
                    },
                    ..Default::default()
                }
            })
            .width(Length::Fill)
    )
    .on_press(AppMessage::SlotSelected(slot_index))
    .into()
}

fn status_badge(state: &SlotState) -> Element<'static, AppMessage> {
    let (status_text, color) = match state {
        SlotState::Idle => ("Idle", iced::Color::from_rgb(0.5, 0.5, 0.5)),
        SlotState::Charging => ("⚡ Charging", iced::Color::from_rgb(0.2, 0.8, 0.2)),
        SlotState::Discharging => ("↓ Discharging", iced::Color::from_rgb(0.8, 0.6, 0.2)),
        SlotState::Completed => ("✓ Done", iced::Color::from_rgb(0.2, 0.6, 1.0)),
        SlotState::Error(_) => ("⚠ Error", iced::Color::from_rgb(0.8, 0.2, 0.2)),
        SlotState::Paused => ("⏸ Paused", iced::Color::from_rgb(0.8, 0.8, 0.2)),
    };
    text(status_text).size(14).color(color).into()
}

fn config_dialog_view<'a>(slot: &'a Slot, config: Option<&'a TaskConfig>) -> Element<'a, AppMessage> {
    let default_config = TaskConfig::default();
    let config = config.unwrap_or(&default_config);

    let chemistry_picker = column![
        text("Battery Chemistry:"),
        pick_list(
            BatteryChemistry::all(),
            Some(config.battery_chemistry),
            move |chem| AppMessage::UpdateSlotChemistry(slot.id, chem)
        ),
    ]
    .spacing(5);

    let task_types = vec![
        TaskType::Charge,
        TaskType::Discharge,
        TaskType::Storage,
        TaskType::Cycle { charge_cycles: 1, discharge_cycles: 1 },
    ];

    let task_type_picker = column![
        text("Operation Mode:"),
        pick_list(
            task_types,
            Some(config.task_type.clone()),
            move |task_type| AppMessage::UpdateSlotTaskType(slot.id, task_type)
        ),
    ]
    .spacing(5);

    let capacity_input = column![
        text("Capacity (mAh):"),
        text_input(
            &format!("{}", config.capacity_limit.unwrap_or(3000)),
            &format!("{}", config.capacity_limit.unwrap_or(3000))
        )
        .on_input(move |val| {
            if let Ok(cap) = val.parse::<u32>() {
                AppMessage::UpdateSlotCapacity(slot.id, cap)
            } else {
                AppMessage::UpdateSlotCapacity(slot.id, 3000)
            }
        }),
    ]
    .spacing(5);

    let current_input = column![
        text("Charge Current (mA):"),
        text_input(
            &format!("{}", config.charge_current_ma),
            &format!("{}", config.charge_current_ma)
        )
        .on_input(move |val| {
            if let Ok(curr) = val.parse::<u16>() {
                AppMessage::UpdateSlotChargeCurrent(slot.id, curr)
            } else {
                AppMessage::UpdateSlotChargeCurrent(slot.id, 1000)
            }
        }),
    ]
    .spacing(5);

    let info_text = column![
        text(format!("Target: {:.2}V", config.target_voltage)).size(12),
        text(format!("Cutoff: {:.2}V", config.cutoff_voltage.unwrap_or(0.0))).size(12),
    ]
    .spacing(3);

    let buttons = row![
        button("Cancel")
            .on_press(AppMessage::CancelSlotConfig(slot.id))
            .style(button::secondary),
        button("Start")
            .on_press(AppMessage::ApplySlotConfig(slot.id))
            .style(button::primary),
    ]
    .spacing(10);

    let content = column![
        text(format!("Configure Slot {}", slot.id.0 + 1)).size(16),
        chemistry_picker,
        task_type_picker,
        capacity_input,
        current_input,
        info_text,
        buttons,
    ]
    .spacing(10)
    .padding(10);

    container(content)
        .style(container::bordered_box)
        .width(Length::Fill)
        .into()
}