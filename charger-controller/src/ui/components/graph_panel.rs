use iced::{
    widget::{canvas, container},
    Element, Length, Size, Point, Color, mouse,
};
use iced::widget::canvas::Text;

use crate::app::AppMessage;
use crate::data::DataLogger;
use crate::slot::{Slot, SlotId};

pub fn view<'a>(data_logger: &'a DataLogger, slots: &'a [Slot; 4], selected_slot: Option<usize>) -> Element<'a, AppMessage> {
    let graph_canvas = canvas(GraphCanvas::new(data_logger, slots, selected_slot))
        .width(Length::Fill)
        .height(Length::Fill);

    container(graph_canvas)
        .style(container::bordered_box)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(5)
        .into()
}

struct GraphCanvas<'a> {
    data_logger: &'a DataLogger,
    slots: &'a [Slot; 4],
    selected_slot: Option<usize>,
}

impl<'a> GraphCanvas<'a> {
    fn new(data_logger: &'a DataLogger, slots: &'a [Slot; 4], selected_slot: Option<usize>) -> Self {
        Self {
            data_logger,
            slots,
            selected_slot,
        }
    }
}

impl<'a> canvas::Program<AppMessage> for GraphCanvas<'a> {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: iced::Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        
        // Define margins for axes
        let margin_left = 60.0;
        let margin_right = 60.0;
        let margin_top = 20.0;
        let margin_bottom = 40.0;
        
        let _graph_width = bounds.width - margin_left - margin_right;
        let _graph_height = bounds.height - margin_top - margin_bottom;
        
        // Draw background
        frame.fill_rectangle(
            Point::ORIGIN,
            bounds.size(),
            Color::from_rgb(0.1, 0.1, 0.1),
        );

        // Calculate global min/max across all slots for auto-scaling
        let (global_min_voltage, global_max_voltage, global_max_current) = self.calculate_global_ranges();

        // Draw grid lines and axes with labels
        self.draw_grid_and_axes(
            &mut frame, 
            bounds.size(),
            margin_left,
            margin_right,
            margin_top,
            margin_bottom,
            global_min_voltage,
            global_max_voltage,
            global_max_current,
        );

        // Draw data only for selected slot (or all if none selected)
        if let Some(selected_idx) = self.selected_slot {
            if selected_idx < self.slots.len() {
                let slot = &self.slots[selected_idx];
                let voltage_color = match selected_idx {
                    0 => Color::from_rgb(1.0, 0.3, 0.3), // Red
                    1 => Color::from_rgb(0.3, 1.0, 0.3), // Green
                    2 => Color::from_rgb(0.3, 0.3, 1.0), // Blue
                    3 => Color::from_rgb(1.0, 1.0, 0.3), // Yellow
                    _ => Color::WHITE,
                };
                
                // Slightly darker/different shade for current
                let current_color = match selected_idx {
                    0 => Color::from_rgb(0.8, 0.1, 0.1), // Darker red
                    1 => Color::from_rgb(0.1, 0.8, 0.1), // Darker green
                    2 => Color::from_rgb(0.1, 0.1, 0.8), // Darker blue
                    3 => Color::from_rgb(0.8, 0.8, 0.1), // Darker yellow
                    _ => Color::from_rgb(0.7, 0.7, 0.7),
                };

                self.draw_slot_data(
                    &mut frame,
                    bounds.size(),
                    slot.id,
                    voltage_color,
                    current_color,
                    global_min_voltage,
                    global_max_voltage,
                    global_max_current,
                    margin_left,
                    margin_right,
                    margin_top,
                    margin_bottom,
                );
            }
        }

        // Draw legend showing selected slot
        self.draw_legend(&mut frame, bounds.size());

        vec![frame.into_geometry()]
    }
}

impl<'a> GraphCanvas<'a> {
    fn calculate_global_ranges(&self) -> (f32, f32, f32) {
        let mut global_min_voltage = f32::INFINITY;
        let mut global_max_voltage = f32::NEG_INFINITY;
        let mut global_max_current = f32::NEG_INFINITY;

        // Iterate through all slots and find global min/max
        for slot in self.slots.iter() {
            let measurements = self.data_logger.get_measurements_for_slot(slot.id);
            
            // Get the most recent 100 measurements
            let recent_measurements: Vec<_> = measurements
                .iter()
                .rev()
                .take(100)
                .rev()
                .collect();

            for measurement in recent_measurements {
                global_min_voltage = global_min_voltage.min(measurement.voltage);
                global_max_voltage = global_max_voltage.max(measurement.voltage);
                global_max_current = global_max_current.max(measurement.current);
            }
        }

        // If no valid data, return defaults
        if global_min_voltage.is_infinite() || global_max_voltage.is_infinite() {
            global_min_voltage = 0.0;
            global_max_voltage = 5.0;
        }
        if global_max_current.is_infinite() {
            global_max_current = 1.0;
        }

        // Add some padding (5%) to the range for better visualization
        let voltage_padding = (global_max_voltage - global_min_voltage) * 0.05;
        global_min_voltage = (global_min_voltage - voltage_padding).max(0.0);
        global_max_voltage = global_max_voltage + voltage_padding;

        let current_padding = global_max_current * 0.05;
        global_max_current = global_max_current + current_padding;

        (global_min_voltage, global_max_voltage, global_max_current)
    }

    fn draw_grid_and_axes(
        &self, 
        frame: &mut canvas::Frame, 
        size: Size,
        margin_left: f32,
        margin_right: f32,
        margin_top: f32,
        margin_bottom: f32,
        min_voltage: f32,
        max_voltage: f32,
        max_current: f32,
    ) {
        use canvas::Path;
        
        let graph_width = size.width - margin_left - margin_right;
        let graph_height = size.height - margin_top - margin_bottom;
        
        let grid_color = Color::from_rgb(0.3, 0.3, 0.3);
        let axis_color = Color::from_rgb(0.6, 0.6, 0.6);
        let text_color = Color::from_rgb(0.8, 0.8, 0.8);
        
        // Draw vertical grid lines (time intervals)
        for i in 0..=10 {
            let x = margin_left + (i as f32 / 10.0) * graph_width;
            let path = Path::line(
                Point::new(x, margin_top), 
                Point::new(x, margin_top + graph_height)
            );
            frame.stroke(&path, canvas::Stroke::default().with_color(grid_color).with_width(1.0));
        }
        
        // Draw horizontal grid lines
        for i in 0..=10 {
            let y = margin_top + (i as f32 / 10.0) * graph_height;
            let path = Path::line(
                Point::new(margin_left, y), 
                Point::new(margin_left + graph_width, y)
            );
            frame.stroke(&path, canvas::Stroke::default().with_color(grid_color).with_width(1.0));
        }
        
        // Draw main axes borders
        let border = Path::rectangle(
            Point::new(margin_left, margin_top),
            Size::new(graph_width, graph_height)
        );
        frame.stroke(&border, canvas::Stroke::default().with_color(axis_color).with_width(2.0));
        
        // Draw voltage scale on left Y-axis (V)
        for i in 0..=5 {
            let y = margin_top + graph_height - (i as f32 / 5.0) * graph_height;
            let voltage_value = min_voltage + (i as f32 / 5.0) * (max_voltage - min_voltage);
            
            let label = format!("{:.2} V", voltage_value);
            let text = Text {
                content: label,
                position: Point::new(5.0, y - 6.0),
                color: text_color,
                size: 12.0.into(),
                ..Default::default()
            };
            frame.fill_text(text);
        }
        
        // Draw "V" label on left axis
        let v_label = Text {
            content: "V".to_string(),
            position: Point::new(45.0, margin_top - 15.0),
            color: Color::from_rgb(1.0, 0.5, 0.5),
            size: 14.0.into(),
            ..Default::default()
        };
        frame.fill_text(v_label);
        
        // Draw current scale on right Y-axis (I)
        for i in 0..=5 {
            let y = margin_top + graph_height - (i as f32 / 5.0) * graph_height;
            let current_value = (i as f32 / 5.0) * max_current;
            
            let label = format!("{:.2} A", current_value);
            let text = Text {
                content: label,
                position: Point::new(size.width - margin_right + 20.0, y - 6.0),
                color: text_color,
                size: 12.0.into(),
                ..Default::default()
            };
            frame.fill_text(text);
        }
        
        // Draw "I" label on right axis
        let i_label = Text {
            content: "I".to_string(),
            position: Point::new(size.width - margin_right + 5.0, margin_top - 15.0),
            color: Color::from_rgb(0.5, 0.5, 1.0),
            size: 14.0.into(),
            ..Default::default()
        };
        frame.fill_text(i_label);

        // Draw time axis label at bottom
        let time_label = Text {
            content: "Time".to_string(),
            position: Point::new(margin_left + graph_width / 2.0 - 20.0, size.height - 25.0),
            color: text_color,
            size: 14.0.into(),
            ..Default::default()
        };
        frame.fill_text(time_label);
    }

    fn draw_slot_data(
        &self,
        frame: &mut canvas::Frame,
        size: Size,
        slot_id: SlotId,
        voltage_color: Color,
        current_color: Color,
        global_min_voltage: f32,
        global_max_voltage: f32,
        global_max_current: f32,
        margin_left: f32,
        margin_right: f32,
        margin_top: f32,
        margin_bottom: f32,
    ) {
        use canvas::Path;
        
        let graph_width = size.width - margin_left - margin_right;
        let graph_height = size.height - margin_top - margin_bottom;
        
        let measurements = self.data_logger.get_measurements_for_slot(slot_id);
        if measurements.len() < 2 {
            return;
        }

        // Get the most recent 100 measurements for better performance
        let recent_measurements: Vec<_> = measurements
            .iter()
            .rev()
            .take(100)
            .rev()
            .collect();

        if recent_measurements.is_empty() {
            return;
        }

        // Use global scaling
        let voltage_range = if global_max_voltage > global_min_voltage {
            global_max_voltage - global_min_voltage
        } else {
            1.0 // Default range if all values are the same
        };

        // Draw voltage as DOTS (uses left Y-axis)
        for (i, measurement) in recent_measurements.iter().enumerate() {
            let x = margin_left + (i as f32 / (recent_measurements.len() - 1) as f32) * graph_width;
            let y = margin_top + graph_height - ((measurement.voltage - global_min_voltage) / voltage_range) * graph_height;
            
            // Draw a small circle for each data point
            let circle_path = Path::circle(Point::new(x, y), 2.5);
            frame.fill(&circle_path, voltage_color);
        }

        // Draw current as SOLID LINE (uses right Y-axis)
        if global_max_current > 0.0 {
            let current_path = Path::new(|builder| {
                for (i, measurement) in recent_measurements.iter().enumerate() {
                    let x = margin_left + (i as f32 / (recent_measurements.len() - 1) as f32) * graph_width;
                    let y = margin_top + graph_height - (measurement.current / global_max_current) * graph_height;
                    
                    if i == 0 {
                        builder.move_to(Point::new(x, y));
                    } else {
                        builder.line_to(Point::new(x, y));
                    }
                }
            });

            frame.stroke(&current_path, canvas::Stroke::default().with_color(current_color).with_width(2.0));
        }
    }

    fn draw_legend(&self, frame: &mut canvas::Frame, size: Size) {
        // Show legend for selected slot only
        if let Some(selected_idx) = self.selected_slot {
            let legend_y = size.height - 30.0;
            let voltage_color = match selected_idx {
                0 => Color::from_rgb(1.0, 0.3, 0.3), // Red
                1 => Color::from_rgb(0.3, 1.0, 0.3), // Green
                2 => Color::from_rgb(0.3, 0.3, 1.0), // Blue
                3 => Color::from_rgb(1.0, 1.0, 0.3), // Yellow
                _ => Color::WHITE,
            };
            
            let current_color = match selected_idx {
                0 => Color::from_rgb(0.8, 0.1, 0.1), 
                1 => Color::from_rgb(0.1, 0.8, 0.1), 
                2 => Color::from_rgb(0.1, 0.1, 0.8), 
                3 => Color::from_rgb(0.8, 0.8, 0.1), 
                _ => Color::from_rgb(0.7, 0.7, 0.7),
            };

            // Draw voltage legend (dots)
            for i in 0..5 {
                let x = 10.0 + (i as f32 * 5.0);
                let circle_path = canvas::Path::circle(Point::new(x, legend_y), 2.5);
                frame.fill(&circle_path, voltage_color);
            }
            
            let voltage_label = Text {
                content: format!("Slot {} - Voltage (V)", selected_idx + 1),
                position: Point::new(40.0, legend_y - 6.0),
                color: voltage_color,
                size: 12.0.into(),
                ..Default::default()
            };
            frame.fill_text(voltage_label);
            
            // Draw current legend (solid line)
            let current_line = canvas::Path::line(
                Point::new(200.0, legend_y),
                Point::new(220.0, legend_y)
            );
            frame.stroke(&current_line, canvas::Stroke::default().with_color(current_color).with_width(2.0));
            
            let current_label = Text {
                content: format!("Slot {} - Current (A)", selected_idx + 1),
                position: Point::new(230.0, legend_y - 6.0),
                color: current_color,
                size: 12.0.into(),
                ..Default::default()
            };
            frame.fill_text(current_label);
        }
    }
}
