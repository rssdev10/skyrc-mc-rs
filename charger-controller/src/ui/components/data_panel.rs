use iced::{
    Element, Length, widget::{button, column, container, row as iced_row, rule, scrollable, space::horizontal, text}
};

use crate::app::AppMessage;
use crate::i18n::t;
use crate::data::DataLogger;

pub fn view(data_logger: &DataLogger, show_detailed_stats: bool) -> Element<'_, AppMessage> {
    let stats = data_logger.get_statistics();
    let aligned_row_count = data_logger.get_time_aligned_data().len();
    
    // Header with measurement count
    let measurement_count = text(format!(
        "{}: {}",
        t!("label.total_samples"),
        stats.total_measurements
    ));
    
    let aligned_info = text(format!(
        "{}: {} ({})",
        t!("label.aligned_rows"),
        aligned_row_count,
        t!("label.for_export")
    )).size(12);

    let time_range = if let (Some(oldest), Some(newest)) = 
        (stats.oldest_measurement, stats.newest_measurement) {
        let duration = newest - oldest;
        text(format!(
            "{}: {}m {}s",
            t!("label.recording_duration"),
            duration.num_minutes(),
            duration.num_seconds() % 60
        ))
    } else {
        text(t!("label.no_data").to_string())
    };

    // Export controls
    let export_controls = iced_row![
        column![
            button(text(t!("btn.export_csv").to_string()))
                .on_press(AppMessage::ExportAllSamples),
            
            button(text(t!("btn.export_aligned").to_string()))
                .on_press(AppMessage::ExportTimeAligned),
        ].spacing(10),
        horizontal(),
        button(text(t!("btn.clear_data").to_string()))
            .on_press(AppMessage::ClearData)
            .style(button::danger),
    ]
    .spacing(10)
    .align_y(iced::alignment::Vertical::Center);

    // Toggle button for event stream log
    let toggle_text = if show_detailed_stats {
        format!("▼ {}", t!("btn.hide_events"))
    } else {
        format!("▶ {}", t!("btn.show_events"))
    };
    let toggle_button = button(text(toggle_text).size(12))
        .on_press(AppMessage::ToggleDetailedStats)
        .style(button::secondary);

    // Build the main content
    let mut content_elements: Vec<Element<AppMessage>> = vec![
        text(t!("label.data_statistics").to_string()).size(16).into(),
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
        let stream_header = text(format!("{}:", t!("label.event_stream"))).size(14);
        
        // Show last 50 individual measurements
        let recent_measurements = data_logger.get_recent_measurements(50);
        let stream_rows = column(
            recent_measurements.iter().rev().map(|m| {
                iced_row![
                    text(m.timestamp.format("%H:%M:%S.%3f").to_string()).size(10).width(Length::Fixed(100.0)),
                    text(format!("{} {}", t!("label.slot"), m.slot_id.0 + 1)).size(10).width(Length::Fixed(50.0)),
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