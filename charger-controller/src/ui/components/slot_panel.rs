use iced::{
    widget::{button, column, container, row, text, progress_bar, pick_list, text_input, mouse_area},
    Element, Length, Border,
};

use crate::app::AppMessage;
use crate::i18n::t;
use crate::slot::{Slot, SlotState, TaskConfig, TaskType, BatteryChemistry};

pub fn view<'a>(slot: &'a Slot, config: Option<&'a TaskConfig>, is_configuring: bool, is_connected: bool, is_selected: bool, slot_index: usize) -> Element<'a, AppMessage> {
    if is_configuring {
        return config_dialog_view(slot, config);
    }

    let slot_header = row![
        text(format!("{} {}", t!("label.slot"), slot.id.0 + 1)).size(16),
        iced::widget::space::horizontal(),
        status_badge(&slot.state),
    ]
    .align_y(iced::Center);

    let measurements = column![
        row![
            column![
                text(t!("label.voltage").to_string()).size(12),
                text(format!("{:.3}V", slot.current_voltage)).size(14),
            ]
            .width(Length::FillPortion(1))
            .align_x(iced::Center),
            column![
                text(t!("label.current").to_string()).size(12),
                text(format!("{}mA", slot.current_current as u16)).size(14),
            ]
            .width(Length::FillPortion(1))
            .align_x(iced::Center),
        ]
        .spacing(10),
        row![
            column![
                text(t!("label.power").to_string()).size(12),
                text(format!("{:.1}W", slot.power_w())).size(14),
            ]
            .width(Length::FillPortion(1))
            .align_x(iced::Center),
            column![
                text(t!("label.capacity").to_string()).size(12),
                text(format!("{}mAh", slot.capacity_mah)).size(14),
            ]
            .width(Length::FillPortion(1))
            .align_x(iced::Center),
        ]
        .spacing(10),
        row![
            column![
                text(t!("label.resistance").to_string()).size(12),
                text(format!("{}mΩ", slot.resistance_milliohm)).size(14),
            ]
            .width(Length::FillPortion(1))
            .align_x(iced::Center),
            column![
                text(t!("label.time").to_string()).size(12),
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
                text(t!("label.progress").to_string()).size(12),
                iced::widget::space::horizontal(),
                text(format!("{:.1}%", slot.get_progress_percentage())).size(12),
            ],
            container(progress_bar(0.0..=1.0, progress_value))
                .height(Length::Fixed(8.0)),
        ]
        .spacing(2)
        .width(Length::Fill);

        let stop_button = button(text(t!("btn.stop").to_string()).size(14))
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
        let mut btn = button(text(t!("btn.configure_start").to_string()).size(14))
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
        let btn = button(text(t!("btn.reset").to_string()).size(14))
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

    // Highlight slots with battery inserted (voltage > 0)
    let has_battery = slot.current_voltage > 0.0;

    mouse_area(
        container(content)
            .style(move |theme: &iced::Theme| {
                let bg = theme.palette().background;
                let is_dark = (bg.r + bg.g + bg.b) / 3.0 < 0.5;
                let background_color = if has_battery {
                    if is_dark {
                        Some(iced::Color::from_rgba(1.0, 1.0, 1.0, 0.08))
                    } else {
                        Some(iced::Color::from_rgba(0.0, 0.6, 0.0, 0.08))
                    }
                } else {
                    None
                };
                container::Style {
                    border: Border {
                        color: border_color,
                        width: border_width,
                        radius: 5.0.into(),
                    },
                    background: background_color.map(iced::Background::Color),
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
        SlotState::Idle => (t!("label.idle").to_string(), iced::Color::from_rgb(0.5, 0.5, 0.5)),
        SlotState::Charging => (format!("⚡ {}", t!("label.charging")), iced::Color::from_rgb(0.2, 0.8, 0.2)),
        SlotState::Discharging => (format!("↓ {}", t!("label.discharging")), iced::Color::from_rgb(0.8, 0.6, 0.2)),
        SlotState::Completed => (format!("✓ {}", t!("label.done")), iced::Color::from_rgb(0.2, 0.6, 1.0)),
        SlotState::Error(_) => (format!("⚠ {}", t!("label.error")), iced::Color::from_rgb(0.8, 0.2, 0.2)),
        SlotState::Paused => (format!("⏸ {}", t!("label.paused")), iced::Color::from_rgb(0.8, 0.8, 0.2)),
    };
    text(status_text).size(14).color(color).into()
}

fn config_dialog_view<'a>(slot: &'a Slot, config: Option<&'a TaskConfig>) -> Element<'a, AppMessage> {
    let default_config = TaskConfig::default();
    let config = config.unwrap_or(&default_config);

    let chemistry_picker = column![
        text(format!("{}:", t!("config.chemistry"))),
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
        text(format!("{}:", t!("config.operation_mode"))),
        pick_list(
            task_types,
            Some(config.task_type.clone()),
            move |task_type| AppMessage::UpdateSlotTaskType(slot.id, task_type)
        ),
    ]
    .spacing(5);

    let capacity_input = column![
        text(format!("{} (mAh):", t!("config.capacity"))),
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
        text(format!("{} (mA):", t!("config.charge_current"))),
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
        button(text(t!("btn.cancel").to_string()))
            .on_press(AppMessage::CancelSlotConfig(slot.id))
            .style(button::secondary),
        button(text(t!("btn.start").to_string()))
            .on_press(AppMessage::ApplySlotConfig(slot.id))
            .style(button::primary),
    ]
    .spacing(10);

    let content = column![
        text(format!("{} {}", t!("config.configure_slot"), slot.id.0 + 1)).size(16),
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