//! Voltage and current graph of the selected slot, on a time axis.
//!
//! # Data ownership
//!
//! The graph owns no data. It draws a view of the [`DataLogger`], the same buffer the CSV
//! exports write. The two time modes differ only in which part of that buffer they show:
//!
//! - **Roll**: the last N seconds, labelled relative to now (`-2m05s` .. `0`). The domain is
//!   the window itself, so a partly filled window draws on the right of the canvas.
//! - **Infinite**: everything recorded, labelled with wall-clock times.
//!
//! **Clear** hides samples older than now from the graph in both modes without deleting
//! them; an export still contains them.
//!
//! # Drawing
//!
//! - Points are placed by timestamp. Both axes scale to the visible samples.
//! - Traces are min/max decimated to at most two points per horizontal pixel, so the whole
//!   span is always drawn and single-sample transients survive.
//! - A pause in the live data breaks the line, so a disconnect shows as a gap. An isolated
//!   reading between two gaps is drawn as a dot.
//! - Samples read from the charger's own history (0x98) are drawn dashed. They have no
//!   current, so the current trace has a gap there.

use chrono::{DateTime, Local, Utc};
use iced::widget::canvas::{self, stroke::LineDash, Frame, Geometry, Path, Stroke, Text};
use iced::widget::{button, canvas as canvas_widget, column, container, row, text, text_input};
use iced::{alignment, mouse, Color, Element, Length, Point, Rectangle, Size, Theme};

use crate::app::AppMessage;
use crate::data::{DataLogger, MeasurementPoint, SampleSource};
use crate::i18n::t;
use crate::settings::GraphTimeMode;
use crate::slot::SlotId;

const MARGIN_LEFT: f32 = 60.0;
const MARGIN_RIGHT: f32 = 60.0;
const MARGIN_TOP: f32 = 20.0;
/// Room for the time labels and the legend.
const MARGIN_BOTTOM: f32 = 44.0;
const GRID_LINES: usize = 4;
/// Smallest axis spans, so a flat trace is not stretched over the whole height.
const MIN_V_SPAN: f32 = 0.1;
const MIN_I_SPAN: f32 = 100.0;
/// The app reads each slot once a second; a longer pause in live data breaks the line.
const LIVE_GAP_MS: i64 = 5_000;
/// Charger history is up to 128 s apart, and stops short of live data by about one
/// interval; only a longer pause breaks its line.
const HISTORY_GAP_MS: i64 = 300_000;
/// Below this width the time axis is labelled at its ends only.
const X_AXIS_FULL_LABELS_W: f32 = 320.0;

/// Bold color of each slot's current trace.
fn slot_current_color(idx: usize) -> Color {
    match idx {
        0 => Color::from_rgb(1.0, 0.25, 0.25), // vivid red
        1 => Color::from_rgb(0.2, 0.95, 0.2),  // vivid green
        2 => Color::from_rgb(0.25, 0.25, 1.0), // vivid blue
        3 => Color::from_rgb(1.0, 1.0, 0.2),   // vivid yellow
        _ => Color::from_rgb(0.8, 0.8, 0.8),
    }
}

/// Color of each slot's voltage trace: the same family as its current, visibly distinct.
fn slot_voltage_color(idx: usize) -> Color {
    match idx {
        0 => Color::from_rgb(1.0, 0.62, 0.45), // salmon / orange-red
        1 => Color::from_rgb(0.45, 1.0, 0.70), // mint green
        2 => Color::from_rgb(0.25, 0.78, 1.0), // sky blue / cyan
        3 => Color::from_rgb(1.0, 0.72, 0.25), // orange-yellow
        _ => Color::from_rgb(0.65, 0.65, 0.65),
    }
}

/// How the graph is to show the data (app state, partly persisted in the settings).
#[derive(Debug, Clone, Copy)]
pub struct GraphOptions<'a> {
    pub mode: GraphTimeMode,
    /// Roll window in seconds.
    pub window_s: u32,
    /// The window field as typed, applied with Set.
    pub window_input: &'a str,
    /// Set by Clear: older samples are not drawn.
    pub epoch: Option<DateTime<Utc>>,
}

pub fn view<'a>(
    data_logger: &'a DataLogger,
    selected_slot: Option<usize>,
    options: GraphOptions<'a>,
) -> Element<'a, AppMessage> {
    let samples = selected_slot.map_or(&[][..], |i| {
        data_logger.get_measurements_for_slot(SlotId(i))
    });
    let chart = canvas_widget(GraphCanvas {
        samples,
        selected_slot,
        options,
        now: Utc::now(),
    })
    .width(Length::Fill)
    .height(Length::Fill);

    let mode_label = match options.mode {
        GraphTimeMode::Roll => t!("graph.mode_roll"),
        GraphTimeMode::Infinite => t!("graph.mode_infinite"),
    };
    let mut toolbar = row![button(text(mode_label.to_string()).size(12))
        .on_press(AppMessage::GraphToggleMode)
        .padding([2, 8])]
    .spacing(6)
    .align_y(iced::Alignment::Center);
    if options.mode == GraphTimeMode::Roll {
        toolbar = toolbar
            .push(text(t!("graph.window_s").to_string()).size(12))
            .push(
                text_input("600", options.window_input)
                    .on_input(AppMessage::GraphWindowInput)
                    .on_submit(AppMessage::GraphWindowApply)
                    .size(12)
                    .width(Length::Fixed(70.0)),
            )
            .push(
                button(text(t!("btn.set").to_string()).size(12))
                    .on_press(AppMessage::GraphWindowApply)
                    .padding([2, 8]),
            );
    }
    toolbar = toolbar.push(
        button(text(t!("graph.clear").to_string()).size(12))
            .on_press(AppMessage::GraphClear)
            .style(button::danger)
            .padding([2, 8]),
    );
    if let (Some(first), Some(last)) = (samples.first(), samples.last()) {
        let span = (last.timestamp - first.timestamp).num_seconds();
        toolbar = toolbar
            .push(text(t!("graph.recorded", span = fmt_duration(span)).to_string()).size(11));
    }

    container(column![chart, toolbar].spacing(4))
        .style(container::bordered_box)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(5)
        .into()
}

struct GraphCanvas<'a> {
    /// The selected slot's samples, oldest first.
    samples: &'a [MeasurementPoint],
    selected_slot: Option<usize>,
    options: GraphOptions<'a>,
    now: DateTime<Utc>,
}

impl canvas::Program<AppMessage> for GraphCanvas<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let bg = theme.palette().background;
        let is_dark = (bg.r + bg.g + bg.b) / 3.0 < 0.5;
        let colors = Palette::new(is_dark);
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), colors.background);
        self.render(&mut frame, bounds.size(), &colors);
        vec![frame.into_geometry()]
    }
}

struct Palette {
    background: Color,
    grid: Color,
    axis: Color,
    text: Color,
}

impl Palette {
    fn new(is_dark: bool) -> Self {
        if is_dark {
            Self {
                background: Color::from_rgb(0.1, 0.1, 0.1),
                grid: Color::from_rgba(1.0, 1.0, 1.0, 0.10),
                axis: Color::from_rgba(1.0, 1.0, 1.0, 0.55),
                text: Color::from_rgb(0.8, 0.8, 0.8),
            }
        } else {
            Self {
                background: Color::from_rgb(0.97, 0.97, 0.97),
                grid: Color::from_rgba(0.0, 0.0, 0.0, 0.15),
                axis: Color::from_rgba(0.0, 0.0, 0.0, 0.55),
                text: Color::from_rgb(0.2, 0.2, 0.2),
            }
        }
    }
}

impl GraphCanvas<'_> {
    fn render(&self, frame: &mut Frame, size: Size, colors: &Palette) {
        let Some(slot) = self.selected_slot else {
            draw_message(frame, size, &t!("graph.select_slot"), colors.text);
            return;
        };
        let cutoff = visible_cutoff(self.now, self.options);
        let visible = visible_samples(self.samples, cutoff);
        if visible.is_empty() {
            draw_message(frame, size, &t!("graph.no_data"), colors.text);
            return;
        }

        let (t_min_ms, span_ms) = time_domain(visible, self.now, self.options.mode, cutoff);
        let ctx = PlotCtx {
            x_off: MARGIN_LEFT,
            y_off: MARGIN_TOP,
            graph_w: (size.width - MARGIN_LEFT - MARGIN_RIGHT).max(1.0),
            graph_h: (size.height - MARGIN_TOP - MARGIN_BOTTOM).max(1.0),
            t_min_ms,
            span_ms,
        };
        draw_grid(frame, &ctx, colors);
        draw_x_axis(frame, &ctx, self.options.mode, colors.text);

        let voltage_color = slot_voltage_color(slot);
        let current_color = slot_current_color(slot);

        let v_range = auto_range(visible.iter().map(|m| m.voltage), MIN_V_SPAN);
        let i_range = auto_range(visible.iter().filter_map(|m| m.current), MIN_I_SPAN);
        // The charger reports currents as positive: keep zero as the floor.
        let i_lowest = visible
            .iter()
            .filter_map(|m| m.current)
            .fold(f32::MAX, f32::min);
        let i_range = (if i_lowest >= 0.0 { 0.0 } else { i_range.0 }, i_range.1);

        draw_trace(
            frame,
            visible,
            &ctx,
            |m| Some(m.voltage),
            v_range,
            voltage_color,
            1.5,
        );
        draw_trace(
            frame,
            visible,
            &ctx,
            |m| m.current,
            i_range,
            current_color,
            2.5,
        );
        draw_y_axis(
            frame,
            &ctx,
            v_range,
            3,
            ctx.x_off - 8.0,
            alignment::Horizontal::Right,
            voltage_color,
        );
        draw_y_axis(
            frame,
            &ctx,
            i_range,
            0,
            ctx.x_off + ctx.graph_w + 8.0,
            alignment::Horizontal::Left,
            current_color,
        );
        // Units on the line of the top values: V left of the voltage scale, mA right of the
        // current scale.
        let unit_y = ctx.y_off - 7.0;
        for (unit, x, align, color) in [
            ("V", 4.0, alignment::Horizontal::Left, voltage_color),
            (
                "mA",
                size.width - 4.0,
                alignment::Horizontal::Right,
                current_color,
            ),
        ] {
            frame.fill_text(Text {
                content: unit.to_string(),
                position: Point::new(x, unit_y),
                color,
                size: 13.0.into(),
                align_x: align.into(),
                ..Default::default()
            });
        }

        let has_history = visible
            .iter()
            .any(|m| m.source == SampleSource::ChargerHistory);
        draw_legend(
            frame,
            size,
            slot,
            voltage_color,
            current_color,
            has_history,
            colors.text,
        );
    }
}

/// Everything a trace needs to map (time, value) onto the canvas.
#[derive(Clone, Copy, Debug)]
struct PlotCtx {
    x_off: f32,
    y_off: f32,
    graph_w: f32,
    graph_h: f32,
    /// Left edge of the time domain, in epoch milliseconds.
    t_min_ms: i64,
    /// Width of the time domain in milliseconds; always >= 1.
    span_ms: f64,
}

impl PlotCtx {
    /// Fraction of the domain at which `t_ms` falls, clamped to `0..=1`.
    fn frac_of(&self, t_ms: i64) -> f64 {
        (((t_ms - self.t_min_ms) as f64) / self.span_ms).clamp(0.0, 1.0)
    }

    fn x_of(&self, t_ms: i64) -> f32 {
        self.x_off + self.frac_of(t_ms) as f32 * self.graph_w
    }
}

/// Oldest timestamp to draw, or `None` for everything. The Clear epoch applies in both
/// modes; Roll also bounds by its window.
fn visible_cutoff(now: DateTime<Utc>, options: GraphOptions<'_>) -> Option<DateTime<Utc>> {
    match options.mode {
        GraphTimeMode::Roll => {
            let window_start = now - chrono::Duration::seconds(i64::from(options.window_s));
            Some(
                options
                    .epoch
                    .map_or(window_start, |epoch| epoch.max(window_start)),
            )
        }
        GraphTimeMode::Infinite => options.epoch,
    }
}

/// The samples at or after `cutoff`. The slice is in time order, so this is a binary
/// search, not a scan: it runs on every redraw.
fn visible_samples(
    samples: &[MeasurementPoint],
    cutoff: Option<DateTime<Utc>>,
) -> &[MeasurementPoint] {
    match cutoff {
        None => samples,
        Some(c) => &samples[samples.partition_point(|m| m.timestamp < c)..],
    }
}

/// The time domain to draw over, as `(t_min_ms, span_ms)`. In Roll it is the window, so
/// the plot moves with time; in Infinite it is the recorded span, so it holds still when
/// no data arrives.
fn time_domain(
    visible: &[MeasurementPoint],
    now: DateTime<Utc>,
    mode: GraphTimeMode,
    cutoff: Option<DateTime<Utc>>,
) -> (i64, f64) {
    let first = visible.first().map_or(now, |m| m.timestamp);
    let last = visible.last().map_or(now, |m| m.timestamp);
    let (start, end) = match mode {
        GraphTimeMode::Roll => (cutoff.unwrap_or(first), last.max(now)),
        GraphTimeMode::Infinite => (first, last),
    };
    let t_min_ms = start.timestamp_millis();
    let span_ms = (end.timestamp_millis() - t_min_ms) as f64;
    (t_min_ms, if span_ms >= 1.0 { span_ms } else { 1000.0 })
}

/// Whether the line between two neighbouring samples is broken.
fn is_gap(a: &MeasurementPoint, b: &MeasurementPoint) -> bool {
    let limit =
        if a.source == SampleSource::ChargerHistory || b.source == SampleSource::ChargerHistory {
            HISTORY_GAP_MS
        } else {
            LIVE_GAP_MS
        };
    (b.timestamp - a.timestamp).num_milliseconds() > limit
}

/// A polyline in canvas x and data value, and whether it shows charger history.
#[derive(Debug, Clone, PartialEq)]
struct Segment {
    points: Vec<(f32, f32)>,
    history: bool,
}

/// One pixel column's worth of samples, reduced to its extremes.
#[derive(Clone, Copy)]
struct Bucket {
    col: usize,
    lo: (f32, f32),
    hi: (f32, f32),
    lo_t: i64,
    hi_t: i64,
}

impl Bucket {
    fn new(col: usize, x: f32, v: f32, t: i64) -> Self {
        Self {
            col,
            lo: (x, v),
            hi: (x, v),
            lo_t: t,
            hi_t: t,
        }
    }

    fn add(&mut self, x: f32, v: f32, t: i64) {
        if v < self.lo.1 {
            self.lo = (x, v);
            self.lo_t = t;
        }
        if v > self.hi.1 {
            self.hi = (x, v);
            self.hi_t = t;
        }
    }

    /// The column's extremes in time order.
    fn flush_into(&self, out: &mut Vec<(f32, f32)>) {
        match self.lo_t.cmp(&self.hi_t) {
            std::cmp::Ordering::Equal => out.push(self.lo),
            std::cmp::Ordering::Less => out.extend([self.lo, self.hi]),
            std::cmp::Ordering::Greater => out.extend([self.hi, self.lo]),
        }
    }
}

/// Reduces `visible` to at most two points per pixel column (each column's minimum and
/// maximum), split into polylines at gaps, at samples without a value and where the
/// source changes between live and history. A history polyline and the live one after it
/// share their boundary point, so the trace stays continuous.
fn decimate(
    visible: &[MeasurementPoint],
    ctx: &PlotCtx,
    value_of: &impl Fn(&MeasurementPoint) -> Option<f32>,
) -> Vec<Segment> {
    let cols = ctx.graph_w.max(1.0) as usize;
    let mut segments = Vec::new();
    let mut seg = Segment {
        points: Vec::new(),
        history: false,
    };
    let mut bucket: Option<Bucket> = None;
    let mut prev: Option<&MeasurementPoint> = None;

    let finish = |seg: &mut Segment, bucket: &mut Option<Bucket>, segments: &mut Vec<Segment>| {
        if let Some(b) = bucket.take() {
            b.flush_into(&mut seg.points);
        }
        if !seg.points.is_empty() {
            segments.push(std::mem::replace(
                seg,
                Segment {
                    points: Vec::new(),
                    history: false,
                },
            ));
        }
    };

    for m in visible {
        let Some(v) = value_of(m) else {
            finish(&mut seg, &mut bucket, &mut segments);
            prev = None;
            continue;
        };
        let history = m.source == SampleSource::ChargerHistory;
        let t = m.timestamp.timestamp_millis();
        let frac = ctx.frac_of(t);
        let col = ((frac * cols as f64) as usize).min(cols - 1);
        let x = ctx.x_off + frac as f32 * ctx.graph_w;

        if let Some(p) = prev {
            if is_gap(p, m) {
                finish(&mut seg, &mut bucket, &mut segments);
            } else if history != seg.history {
                finish(&mut seg, &mut bucket, &mut segments);
                // Start the new polyline where the old one ended.
                if let Some(&last) = segments.last().and_then(|s| s.points.last()) {
                    seg.points.push(last);
                }
            }
        }
        if bucket.is_none() {
            // A new polyline (possibly holding only the boundary point): its first sample
            // decides how it is drawn.
            seg.history = history;
        }

        match bucket.as_mut() {
            Some(b) if b.col == col => b.add(x, v, t),
            _ => {
                if let Some(b) = bucket.take() {
                    b.flush_into(&mut seg.points);
                }
                bucket = Some(Bucket::new(col, x, v, t));
            }
        }
        prev = Some(m);
    }
    finish(&mut seg, &mut bucket, &mut segments);
    segments
}

fn draw_trace(
    frame: &mut Frame,
    visible: &[MeasurementPoint],
    ctx: &PlotCtx,
    value_of: impl Fn(&MeasurementPoint) -> Option<f32>,
    (axis_min, axis_max): (f32, f32),
    color: Color,
    width: f32,
) {
    let range = (axis_max - axis_min).max(1e-9);
    let y_of = |v: f32| {
        let norm = ((v - axis_min) / range).clamp(0.0, 1.0);
        ctx.y_off + ctx.graph_h - norm * ctx.graph_h
    };
    static DASH: &[f32] = &[6.0, 4.0];
    for seg in decimate(visible, ctx, &value_of) {
        if seg.points.len() == 1 {
            // A reading isolated between two gaps: mark it so it is still visible.
            let (x, v) = seg.points[0];
            frame.fill(&Path::circle(Point::new(x, y_of(v)), width.max(2.0)), color);
            continue;
        }
        let path = Path::new(|builder| {
            for (i, &(x, v)) in seg.points.iter().enumerate() {
                let p = Point::new(x, y_of(v));
                if i == 0 {
                    builder.move_to(p);
                } else {
                    builder.line_to(p);
                }
            }
        });
        let stroke = Stroke::default().with_color(color).with_width(width);
        if seg.history {
            frame.stroke(
                &path,
                Stroke {
                    line_dash: LineDash {
                        segments: DASH,
                        offset: 0,
                    },
                    ..stroke
                },
            );
        } else {
            frame.stroke(&path, stroke);
        }
    }
}

fn draw_message(frame: &mut Frame, size: Size, message: &str, color: Color) {
    frame.fill_text(Text {
        content: message.to_string(),
        position: Point::new(size.width / 2.0, size.height / 2.0),
        color,
        size: 14.0.into(),
        align_x: alignment::Horizontal::Center.into(),
        ..Default::default()
    });
}

fn draw_grid(frame: &mut Frame, ctx: &PlotCtx, colors: &Palette) {
    let stroke = Stroke::default().with_color(colors.grid).with_width(1.0);
    for i in 0..=GRID_LINES {
        let frac = i as f32 / GRID_LINES as f32;
        let y = ctx.y_off + frac * ctx.graph_h;
        frame.stroke(
            &Path::line(
                Point::new(ctx.x_off, y),
                Point::new(ctx.x_off + ctx.graph_w, y),
            ),
            stroke,
        );
        let x = ctx.x_off + frac * ctx.graph_w;
        frame.stroke(
            &Path::line(
                Point::new(x, ctx.y_off),
                Point::new(x, ctx.y_off + ctx.graph_h),
            ),
            stroke,
        );
    }
    let border = Path::rectangle(
        Point::new(ctx.x_off, ctx.y_off),
        Size::new(ctx.graph_w, ctx.graph_h),
    );
    frame.stroke(
        &border,
        Stroke::default().with_color(colors.axis).with_width(1.5),
    );
}

fn draw_y_axis(
    frame: &mut Frame,
    ctx: &PlotCtx,
    (min, max): (f32, f32),
    decimals: usize,
    x: f32,
    align: alignment::Horizontal,
    color: Color,
) {
    for i in 0..=GRID_LINES {
        let frac = i as f32 / GRID_LINES as f32;
        frame.fill_text(Text {
            content: format!("{:.*}", decimals, max - frac * (max - min)),
            position: Point::new(x, ctx.y_off + frac * ctx.graph_h - 6.0),
            color,
            size: 11.0.into(),
            align_x: align.into(),
            ..Default::default()
        });
    }
}

/// Labels the time domain under the plot: Roll relative to now, Infinite wall-clock.
fn draw_x_axis(frame: &mut Frame, ctx: &PlotCtx, mode: GraphTimeMode, color: Color) {
    let ticks = if ctx.graph_w >= X_AXIS_FULL_LABELS_W {
        GRID_LINES
    } else {
        1
    };
    let y = ctx.y_off + ctx.graph_h + 4.0;
    for i in 0..=ticks {
        let frac = i as f64 / ticks as f64;
        let t_ms = ctx.t_min_ms + (ctx.span_ms * frac) as i64;
        let content = match mode {
            GraphTimeMode::Roll => fmt_offset((ctx.span_ms * (1.0 - frac)) as i64 / 1000),
            GraphTimeMode::Infinite => DateTime::from_timestamp_millis(t_ms)
                .map(|t| t.with_timezone(&Local).format("%H:%M:%S").to_string())
                .unwrap_or_default(),
        };
        let (x, align) = if i == 0 {
            (ctx.x_off, alignment::Horizontal::Left)
        } else if i == ticks {
            (ctx.x_off + ctx.graph_w, alignment::Horizontal::Right)
        } else {
            (ctx.x_of(t_ms), alignment::Horizontal::Center)
        };
        frame.fill_text(Text {
            content,
            position: Point::new(x, y),
            color,
            size: 10.0.into(),
            align_x: align.into(),
            ..Default::default()
        });
    }
}

fn draw_legend(
    frame: &mut Frame,
    size: Size,
    slot: usize,
    voltage_color: Color,
    current_color: Color,
    has_history: bool,
    text_color: Color,
) {
    let y = size.height - 8.0;
    let mut x = MARGIN_LEFT;
    let mut entry = |frame: &mut Frame, label: String, color: Color, width: f32, dashed: bool| {
        static DASH: &[f32] = &[6.0, 4.0];
        let line = Path::line(Point::new(x, y), Point::new(x + 22.0, y));
        let stroke = Stroke::default().with_color(color).with_width(width);
        let stroke = if dashed {
            Stroke {
                line_dash: LineDash {
                    segments: DASH,
                    offset: 0,
                },
                ..stroke
            }
        } else {
            stroke
        };
        frame.stroke(&line, stroke);
        let chars = label.chars().count() as f32;
        frame.fill_text(Text {
            content: label,
            position: Point::new(x + 27.0, y - 6.0),
            color: if dashed { text_color } else { color },
            size: 11.0.into(),
            ..Default::default()
        });
        x += 27.0 + chars * 6.5 + 16.0;
    };
    entry(
        frame,
        t!("graph.legend_voltage", slot = slot + 1).to_string(),
        voltage_color,
        1.5,
        false,
    );
    entry(
        frame,
        t!("graph.legend_current", slot = slot + 1).to_string(),
        current_color,
        2.5,
        false,
    );
    if has_history {
        entry(
            frame,
            t!("graph.legend_history").to_string(),
            voltage_color,
            1.5,
            true,
        );
    }
}

/// The values' range with 5 % padding, at least `min_span` wide; `(0, min_span)` without
/// values.
fn auto_range(values: impl Iterator<Item = f32>, min_span: f32) -> (f32, f32) {
    let (lo, hi) = values.fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
    if lo > hi {
        return (0.0, min_span);
    }
    let span = hi - lo;
    let pad = span.max(min_span) * 0.05;
    if span >= min_span {
        (lo - pad, hi + pad)
    } else {
        let mid = (lo + hi) / 2.0;
        (mid - min_span / 2.0 - pad, mid + min_span / 2.0 + pad)
    }
}

/// "How long ago" label, e.g. `-1h02m`, `-2m05s`, `-45s`, `0`.
fn fmt_offset(secs: i64) -> String {
    if secs <= 0 {
        "0".to_string()
    } else {
        format!("-{}", fmt_duration(secs))
    }
}

/// Compact duration, e.g. `1h02m`, `2m05s`, `45s`.
fn fmt_duration(secs: i64) -> String {
    let secs = secs.max(0);
    if secs >= 3600 {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(s: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 26, 10, 0, 0).unwrap() + chrono::Duration::seconds(s)
    }

    fn live(s: i64, v: f32) -> MeasurementPoint {
        MeasurementPoint {
            timestamp: at(s),
            slot_id: SlotId(3),
            voltage: v,
            current: Some(500.0),
            state: String::new(),
            mode: String::new(),
            resistance_milliohm: 0,
            elapsed_seconds: 0,
            source: SampleSource::Live,
        }
    }

    fn history(s: i64, v: f32) -> MeasurementPoint {
        MeasurementPoint {
            current: None,
            source: SampleSource::ChargerHistory,
            ..live(s, v)
        }
    }

    fn ctx(t0: i64, t1: i64, width: f32) -> PlotCtx {
        PlotCtx {
            x_off: 0.0,
            y_off: 0.0,
            graph_w: width,
            graph_h: 100.0,
            t_min_ms: at(t0).timestamp_millis(),
            span_ms: (at(t1) - at(t0)).num_milliseconds() as f64,
        }
    }

    #[test]
    fn decimation_keeps_extremes_and_at_most_two_points_per_pixel() {
        let samples: Vec<_> = (0..1000)
            .map(|s| live(s, if s == 500 { 9.0 } else { 4.0 }))
            .collect();
        let segs = decimate(&samples, &ctx(0, 999, 100.0), &|m| Some(m.voltage));
        assert_eq!(segs.len(), 1);
        assert!(segs[0].points.len() <= 200);
        assert!(
            segs[0].points.iter().any(|&(_, v)| v == 9.0),
            "a one-sample spike survives"
        );
    }

    #[test]
    fn a_pause_in_live_data_breaks_the_line() {
        let samples = vec![live(0, 4.0), live(1, 4.0), live(60, 4.1), live(61, 4.1)];
        let segs = decimate(&samples, &ctx(0, 61, 1000.0), &|m| Some(m.voltage));
        assert_eq!(segs.len(), 2);
    }

    #[test]
    fn history_is_its_own_dashed_segment_joined_to_live_data() {
        // History every 8 s, then live data from 30 s.
        let mut samples: Vec<_> = (1..=3).map(|i| history(i * 8, 4.1)).collect();
        samples.extend((30..40).map(|s| live(s, 4.2)));
        let segs = decimate(&samples, &ctx(0, 40, 1000.0), &|m| Some(m.voltage));
        assert_eq!(
            segs.iter().map(|s| s.history).collect::<Vec<_>>(),
            [true, false]
        );
        assert_eq!(segs[0].points.last(), segs[1].points.first(), "continuous");
    }

    #[test]
    fn current_has_a_gap_where_only_history_has_data() {
        let mut samples: Vec<_> = (1..=3).map(|i| history(i * 8, 4.1)).collect();
        samples.extend((30..40).map(|s| live(s, 4.2)));
        let segs = decimate(&samples, &ctx(0, 40, 1000.0), &|m| m.current);
        assert_eq!(segs.len(), 1);
        assert!(!segs[0].history);
    }

    #[test]
    fn roll_shows_the_window_and_clear_hides_older_samples() {
        let samples: Vec<_> = (0..100).map(|s| live(s, 4.0)).collect();
        let mut options = GraphOptions {
            mode: GraphTimeMode::Roll,
            window_s: 10,
            window_input: "10",
            epoch: None,
        };
        let cutoff = visible_cutoff(at(100), options);
        assert_eq!(visible_samples(&samples, cutoff).len(), 10);
        options.mode = GraphTimeMode::Infinite;
        assert_eq!(
            visible_samples(&samples, visible_cutoff(at(100), options)).len(),
            100
        );
        options.epoch = Some(at(95));
        assert_eq!(
            visible_samples(&samples, visible_cutoff(at(100), options)).len(),
            5
        );
    }

    #[test]
    fn axis_ranges_have_a_minimum_span() {
        let (lo, hi) = auto_range([4.198_f32, 4.198].into_iter(), MIN_V_SPAN);
        assert!(hi - lo >= MIN_V_SPAN && lo < 4.198 && hi > 4.198);
        assert_eq!(auto_range(std::iter::empty(), 1.0), (0.0, 1.0));
    }

    #[test]
    fn durations_are_compact() {
        assert_eq!(fmt_duration(45), "45s");
        assert_eq!(fmt_duration(125), "2m05s");
        assert_eq!(fmt_duration(3720), "1h02m");
        assert_eq!(fmt_offset(0), "0");
    }
}
