use iced::{
    widget::{button, column, container, row as iced_row, rule, scrollable, text},
    Element, Length,
};

use crate::app::AppMessage;
use crate::data::DataLogger;

pub fn view(data_logger: &DataLogger, show_detailed_stats: bool) -> Element<'_, AppMessage> {
    let stats = data_logger.get_statistics();
    let aligned_row_count = data_logger.get_time_aligned_data().len();
    
    // Header with measurement count
    let measurement_count = text(format!(
        "Total samples: {}",
        stats.total_measurements
    ));
    
    let aligned_info = text(format!(
        "Time-aligned rows: {} (for export)",
        aligned_row_count
    )).size(12);

    let time_range = if let (Some(oldest), Some(newest)) = 
        (stats.oldest_measurement, stats.newest_measurement) {
        let duration = newest - oldest;
        text(format!(
            "Recording duration: {}m {}s",
            duration.num_minutes(),
            duration.num_seconds() % 60
        ))
    } else {
        text("No data recorded")
    };

    // Export controls
    let export_controls = iced_row![
        button("Export CSV")
            .on_press(AppMessage::ExportAllSamples),
        button("Export Time-Aligned")
            .on_press(AppMessage::ExportTimeAligned),
        button("Clear Data")
            .on_press(AppMessage::ClearData)
            .style(button::danger),
    ]
    .spacing(10);

    // Toggle button for event stream log
    let toggle_text = if show_detailed_stats { "▼ Hide Event Stream" } else { "▶ Show Event Stream" };
    let toggle_button = button(text(toggle_text).size(12))
        .on_press(AppMessage::ToggleDetailedStats)
        .style(button::secondary);

    // Build the main content
    let mut content_elements: Vec<Element<AppMessage>> = vec![
        text("Data Statistics").size(16).into(),
        measurement_count.into(),
        aligned_info.into(),
        time_range.into(),
        rule::horizontal(1).into(),
        export_controls.into(),
        rule::horizontal(1).into(),
        toggle_button.into(),
    ];

    // Event stream log (hidden by default, shown when toggled)
    if show_detailed_stats {
        let stream_header = text("Incoming Event Stream:").size(14);
        
        // Show last 50 individual measurements
        let recent_measurements = data_logger.get_recent_measurements(50);
        let stream_rows = column(
            recent_measurements.iter().rev().map(|m| {
                iced_row![
                    text(m.timestamp.format("%H:%M:%S.%3f").to_string()).size(10).width(Length::Fixed(100.0)),
                    text(format!("Slot {}", m.slot_id.0 + 1)).size(10).width(Length::Fixed(50.0)),
                    text(format!("{:.3}V", m.voltage)).size(10).width(Length::Fixed(60.0)),
                    text(format!("{:.0}mA", m.current * 1000.0)).size(10).width(Length::Fixed(60.0)),
                    text(&m.state).size(10).width(Length::Fixed(80.0)),
                ]
                .spacing(5)
                .into()
            })
            .collect::<Vec<Element<AppMessage>>>()
        )
        .spacing(1);
        
        content_elements.push(stream_header.into());
        content_elements.push(
            scrollable(stream_rows)
                .height(Length::Fixed(200.0))
                .into()
        );
    }

    container(
        column(content_elements)
            .spacing(8)
            .padding(15)
    )
    .style(container::bordered_box)
    .width(Length::FillPortion(1))
    .into()
}