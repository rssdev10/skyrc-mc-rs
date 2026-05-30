use iced::{
    widget::{canvas, container},
    Element, Length, Size, Point, Color, mouse,
};
use iced::widget::canvas::Text;

use crate::app::AppMessage;
use crate::data::DataLogger;
use crate::slot::{Slot, SlotId};

/// Bold base color used for the current (I) line of each slot.
fn slot_current_color(idx: usize) -> Color {
    match idx {
        0 => Color::from_rgb(1.0, 0.25, 0.25), // vivid red
        1 => Color::from_rgb(0.2, 0.95, 0.2),  // vivid green
        2 => Color::from_rgb(0.25, 0.25, 1.0), // vivid blue
        3 => Color::from_rgb(1.0, 1.0, 0.2),   // vivid yellow
        _ => Color::from_rgb(0.8, 0.8, 0.8),
    }
}

/// Slightly shifted color used for the voltage (V) dashed line of each slot.
/// Visually distinct from the current color while staying in the same family.
fn slot_voltage_color(idx: usize) -> Color {
    match idx {
        0 => Color::from_rgb(1.0, 0.62, 0.45), // salmon / orange-red
        1 => Color::from_rgb(0.45, 1.0, 0.70), // mint green
        2 => Color::from_rgb(0.25, 0.78, 1.0), // sky blue / cyan
        3 => Color::from_rgb(1.0, 0.72, 0.25), // orange-yellow
        _ => Color::from_rgb(0.65, 0.65, 0.65),
    }
}

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
        theme: &iced::Theme,
        bounds: iced::Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        
        // Detect if theme is dark or light
        let bg = theme.palette().background;
        let is_dark = (bg.r + bg.g + bg.b) / 3.0 < 0.5;
        
        // Define margins for axes
        let margin_left = 60.0;
        let margin_right = 60.0;
        let margin_top = 20.0;
        let margin_bottom = 40.0;
        
        let _graph_width = bounds.width - margin_left - margin_right;
        let _graph_height = bounds.height - margin_top - margin_bottom;
        
        // Draw background based on theme
        let bg_color = if is_dark {
            Color::from_rgb(0.1, 0.1, 0.1)
        } else {
            Color::from_rgb(0.97, 0.97, 0.97)
        };
        frame.fill_rectangle(
            Point::ORIGIN,
            bounds.size(),
            bg_color,
        );

        // Calculate global min/max across all slots for auto-scaling
        let (global_min_voltage, global_max_voltage, global_max_current) = self.calculate_global_ranges();

        // Axis label colors come from the selected slot's palette
        let (voltage_label_color, current_label_color) = if let Some(idx) = self.selected_slot {
            (slot_voltage_color(idx), slot_current_color(idx))
        } else {
            (Color::from_rgb(0.8, 0.5, 0.3), Color::from_rgb(0.4, 0.6, 1.0))
        };

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
            is_dark,
            voltage_label_color,
            current_label_color,
        );

        // Draw data only for selected slot (or all if none selected)
        if let Some(selected_idx) = self.selected_slot {
            if selected_idx < self.slots.len() {
                let slot = &self.slots[selected_idx];
                let current_color = slot_current_color(selected_idx);
                let voltage_color = slot_voltage_color(selected_idx);

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
        self.draw_legend(&mut frame, bounds.size(), margin_left, margin_right, is_dark);

        vec![frame.into_geometry()]
    }
}

impl<'a> GraphCanvas<'a> {
    fn calculate_global_ranges(&self) -> (f32, f32, f32) {
        let mut global_min_voltage = f32::INFINITY;
        let mut global_max_voltage = f32::NEG_INFINITY;
        let mut global_max_current = f32::NEG_INFINITY;

        // Only use data from the selected slot for scaling
        let slots_to_check: Vec<&Slot> = if let Some(idx) = self.selected_slot {
            self.slots.get(idx).into_iter().collect()
        } else {
            self.slots.iter().collect()
        };

        for slot in slots_to_check {
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
        global_max_voltage += voltage_padding;

        let current_padding = global_max_current * 0.05;
        global_max_current += current_padding;

        (global_min_voltage, global_max_voltage, global_max_current)
    }

    #[allow(clippy::too_many_arguments)]
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
        is_dark: bool,
        voltage_label_color: Color,
        current_label_color: Color,
    ) {
        use canvas::Path;
        
        let graph_width = size.width - margin_left - margin_right;
        let graph_height = size.height - margin_top - margin_bottom;
        
        let grid_color = if is_dark {
            Color::from_rgba(1.0, 1.0, 1.0, 0.10)
        } else {
            Color::from_rgba(0.0, 0.0, 0.0, 0.15)
        };
        let axis_color = if is_dark {
            Color::from_rgba(1.0, 1.0, 1.0, 0.55)
        } else {
            Color::from_rgba(0.0, 0.0, 0.0, 0.55)
        };
        let text_color = if is_dark {
            Color::from_rgb(0.8, 0.8, 0.8)
        } else {
            Color::from_rgb(0.2, 0.2, 0.2)
        };
        
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
        
        // Draw voltage scale on left Y-axis (V) — use voltage_label_color for values
        for i in 0..=5 {
            let y = margin_top + graph_height - (i as f32 / 5.0) * graph_height;
            let voltage_value = min_voltage + (i as f32 / 5.0) * (max_voltage - min_voltage);
            
            let label = format!("{:.3}", voltage_value);
            let text = Text {
                content: label,
                position: Point::new(5.0, y - 6.0),
                color: text_color,
                size: 12.0.into(),
                ..Default::default()
            };
            frame.fill_text(text);
        }
        
        // Draw "V" label on left axis — colored like voltage line
        let v_label = Text {
            content: "V".to_string(),
            position: Point::new(45.0, margin_top - 15.0),
            color: voltage_label_color,
            size: 14.0.into(),
            ..Default::default()
        };
        frame.fill_text(v_label);
        
        // Draw current scale on right Y-axis (I)
        for i in 0..=5 {
            let y = margin_top + graph_height - (i as f32 / 5.0) * graph_height;
            let current_value = (i as f32 / 5.0) * max_current;
            
            let label = format!("{:.2}", current_value);
            let text = Text {
                content: label,
                position: Point::new(size.width - margin_right + 20.0, y - 6.0),
                color: text_color,
                size: 12.0.into(),
                ..Default::default()
            };
            frame.fill_text(text);
        }
        
        // Draw "I" label on right axis — colored like current line
        let i_label = Text {
            content: "I".to_string(),
            position: Point::new(size.width - margin_right + 5.0, margin_top - 15.0),
            color: current_label_color,
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

    #[allow(clippy::too_many_arguments)]
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
        use iced::widget::canvas::stroke::LineDash;
        
        let graph_width = size.width - margin_left - margin_right;
        let graph_height = size.height - margin_top - margin_bottom;
        
        let measurements = self.data_logger.get_measurements_for_slot(slot_id);
        if measurements.len() < 2 {
            return;
        }

        let recent_measurements: Vec<_> = measurements
            .iter()
            .rev()
            .take(100)
            .rev()
            .collect();

        if recent_measurements.is_empty() {
            return;
        }

        let voltage_range = if global_max_voltage > global_min_voltage {
            global_max_voltage - global_min_voltage
        } else {
            1.0
        };

        let n = recent_measurements.len();

        // --- Voltage: thin solid line ---
        let voltage_solid_path = Path::new(|builder| {
            for (i, m) in recent_measurements.iter().enumerate() {
                let x = margin_left + (i as f32 / (n - 1) as f32) * graph_width;
                let y = margin_top + graph_height
                    - ((m.voltage - global_min_voltage) / voltage_range) * graph_height;
                if i == 0 { builder.move_to(Point::new(x, y)); }
                else       { builder.line_to(Point::new(x, y)); }
            }
        });
        frame.stroke(
            &voltage_solid_path,
            canvas::Stroke::default().with_color(voltage_color).with_width(1.0),
        );

        // --- Voltage: dashed overlay ---
        static DASH: &[f32] = &[8.0, 5.0];
        let voltage_dash_path = Path::new(|builder| {
            for (i, m) in recent_measurements.iter().enumerate() {
                let x = margin_left + (i as f32 / (n - 1) as f32) * graph_width;
                let y = margin_top + graph_height
                    - ((m.voltage - global_min_voltage) / voltage_range) * graph_height;
                if i == 0 { builder.move_to(Point::new(x, y)); }
                else       { builder.line_to(Point::new(x, y)); }
            }
        });
        frame.stroke(
            &voltage_dash_path,
            canvas::Stroke {
                line_dash: LineDash { segments: DASH, offset: 0 },
                ..canvas::Stroke::default().with_color(voltage_color).with_width(1.5)
            },
        );

        // --- Voltage: small circles at each data point ---
        for (i, m) in recent_measurements.iter().enumerate() {
            let x = margin_left + (i as f32 / (n - 1) as f32) * graph_width;
            let y = margin_top + graph_height
                - ((m.voltage - global_min_voltage) / voltage_range) * graph_height;
            let circle = Path::circle(Point::new(x, y), 2.5);
            frame.fill(&circle, voltage_color);
        }

        // --- Current: bold solid line ---
        if global_max_current > 0.0 {
            let current_path = Path::new(|builder| {
                for (i, m) in recent_measurements.iter().enumerate() {
                    let x = margin_left + (i as f32 / (n - 1) as f32) * graph_width;
                    let y = margin_top + graph_height
                        - (m.current / global_max_current) * graph_height;
                    if i == 0 { builder.move_to(Point::new(x, y)); }
                    else       { builder.line_to(Point::new(x, y)); }
                }
            });
            frame.stroke(
                &current_path,
                canvas::Stroke::default().with_color(current_color).with_width(2.5),
            );
        }
    }

    fn draw_legend(
        &self,
        frame: &mut canvas::Frame,
        size: Size,
        margin_left: f32,
        margin_right: f32,
        _is_dark: bool,
    ) {
        use iced::advanced::text::Alignment as TextAlign;
        use iced::widget::canvas::stroke::LineDash;

        if let Some(selected_idx) = self.selected_slot {
            let current_color = slot_current_color(selected_idx);
            let voltage_color = slot_voltage_color(selected_idx);

            let legend_y = size.height - 30.0;
            let sample_len = 25.0_f32;
            let gap = 6.0_f32;

            // --- Voltage legend: anchored at left axis + 5px ---
            // Layout: [sample dashed line + dots]  [label text →]
            let v_line_x0 = margin_left + 5.0;
            let v_line_x1 = v_line_x0 + sample_len;

            static DASH: &[f32] = &[8.0, 5.0];
            let dash_line = canvas::Path::line(
                Point::new(v_line_x0, legend_y),
                Point::new(v_line_x1, legend_y),
            );
            frame.stroke(
                &dash_line,
                canvas::Stroke {
                    line_dash: LineDash { segments: DASH, offset: 0 },
                    ..canvas::Stroke::default().with_color(voltage_color).with_width(1.5)
                },
            );
            for dot_x in [v_line_x0, v_line_x0 + sample_len / 2.0, v_line_x1] {
                let circle = canvas::Path::circle(Point::new(dot_x, legend_y), 2.5);
                frame.fill(&circle, voltage_color);
            }
            let voltage_label = Text {
                content: format!("Slot {}  -  Voltage (V)", selected_idx + 1),
                position: Point::new(v_line_x1 + gap, legend_y - 6.0),
                color: voltage_color,
                size: 12.0.into(),
                align_x: TextAlign::Left,
                ..Default::default()
            };
            frame.fill_text(voltage_label);

            // --- Current legend: anchored at right axis − 5px ---
            // Layout: [← label text]  [bold sample line]
            let i_line_x1 = size.width - margin_right - 5.0;
            let i_line_x0 = i_line_x1 - sample_len;

            let current_line = canvas::Path::line(
                Point::new(i_line_x0, legend_y),
                Point::new(i_line_x1, legend_y),
            );
            frame.stroke(
                &current_line,
                canvas::Stroke::default().with_color(current_color).with_width(2.5),
            );
            let current_label = Text {
                content: format!("Slot {}  -  Current (mA)", selected_idx + 1),
                position: Point::new(i_line_x0 - gap, legend_y - 6.0),
                color: current_color,
                size: 12.0.into(),
                align_x: TextAlign::Right,
                ..Default::default()
            };
            frame.fill_text(current_label);
        }
    }
}
