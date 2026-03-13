mod gpu_widget;
mod layer;
mod style;

use std::cell::RefCell;
use std::f64::consts::PI;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use cavaii_common::config::{
    AppConfig, ColorOrientation, RgbaColor, VisualizerConfig, VisualizerType,
};
use cavaii_engine::live::LiveFrameStream;
use tracing::{error, info};

use crate::ui::gpu_widget::BarsWidget;

pub fn spawn_frame_stream(config: &AppConfig) -> Rc<LiveFrameStream> {
    let stream = Rc::new(LiveFrameStream::spawn(config.visualizer.clone()));
    info!("cavaii: using {:?} frame source", stream.source_kind());
    stream
}

pub fn build_overlay_windows(
    app: &gtk::Application,
    config: AppConfig,
    stream: Rc<LiveFrameStream>,
) -> Vec<gtk::ApplicationWindow> {
    style::install_css();

    let monitors = layer::selected_monitors(&config.overlay);
    if monitors.is_empty() {
        return vec![build_overlay_window(app, &config, Rc::clone(&stream), None)];
    }

    monitors
        .into_iter()
        .map(|monitor| build_overlay_window(app, &config, Rc::clone(&stream), Some(monitor)))
        .collect()
}

fn build_overlay_window(
    app: &gtk::Application,
    config: &AppConfig,
    stream: Rc<LiveFrameStream>,
    monitor: Option<gtk::gdk::Monitor>,
) -> gtk::ApplicationWindow {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Cavaii")
        .build();

    window.set_widget_name("cavaii-overlay");
    window.set_decorated(false);
    window.set_resizable(false);
    window.set_focusable(false);

    let bar_count = config.visualizer.bars.max(1);
    let bar_values = Rc::new(RefCell::new(vec![0.0_f64; bar_count]));
    let (widget, render_target) = if config.visualizer.gpu {
        let bars_widget = build_gpu_widget(config, Rc::clone(&bar_values));
        let widget: gtk::Widget = bars_widget.upcast();
        let render_target = RenderTarget(widget.downgrade());
        (widget, render_target)
    } else {
        let drawing_area = build_cpu_area(config, Rc::clone(&bar_values));
        let widget: gtk::Widget = drawing_area.upcast();
        let render_target = RenderTarget(widget.downgrade());
        (widget, render_target)
    };

    window.set_child(Some(&widget));

    layer::apply_default_size(&window, config, monitor.as_ref());
    layer::configure_layer_shell(&window, config, monitor.as_ref());

    attach_frame_tick(config, stream, bar_values, render_target);

    window.present();
    window
}

fn build_cpu_area(config: &AppConfig, bar_values: Rc<RefCell<Vec<f64>>>) -> gtk::DrawingArea {
    let drawing_area = gtk::DrawingArea::new();
    drawing_area.set_widget_name("cavaii-bars");
    drawing_area.set_can_target(false);
    drawing_area.set_size_request(
        to_i32(config.overlay.width),
        to_i32(config.overlay.height),
    );

    let bar_width = f64::from(config.visualizer.bar_width.max(1));
    let bar_corner_radius = f64::from(config.visualizer.bar_corner_radius.max(0.0));
    let wave_thickness = f64::from(config.visualizer.wave_thickness.max(1));
    let gap = f64::from(config.visualizer.gap);
    let gradient = resolve_gradient(&config.visualizer);
    let orientation = config.visualizer.color_orientation;
    let visualizer_type = config.visualizer.visualizer_type;
    let fade = config.visualizer.color_fade;
    let layout_cache = Rc::new(RefCell::new(BarLayoutCache::default()));
    let wave_samples = Rc::new(RefCell::new(Vec::<f64>::new()));
    let wave_points = Rc::new(RefCell::new(Vec::<(f64, f64)>::new()));

    let values_for_draw = Rc::clone(&bar_values);
    let layout_for_draw = Rc::clone(&layout_cache);
    let wave_samples_for_draw = Rc::clone(&wave_samples);
    let wave_points_for_draw = Rc::clone(&wave_points);
    drawing_area.set_draw_func(move |_, ctx, width, height| {
        let values = values_for_draw.borrow();
        if values.is_empty() || width <= 0 || height <= 0 {
            return;
        }

        if visualizer_type == VisualizerType::Wave {
            let mut sampled_scratch = wave_samples_for_draw.borrow_mut();
            let sampled = downsample_wave_values(&values, width, &mut sampled_scratch);
            let mut points_scratch = wave_points_for_draw.borrow_mut();
            set_paint_source(ctx, width, height, &gradient, orientation, fade);
            draw_wave_line(
                ctx,
                width,
                height,
                sampled,
                wave_thickness,
                &mut points_scratch,
            );
            if ctx.stroke().is_err() {
                error!("cavaii: cairo stroke failed");
            }
            return;
        }

        let mut layout = layout_for_draw.borrow_mut();
        layout.update(width, values.len(), bar_width, gap);

        set_paint_source(ctx, width, height, &gradient, orientation, fade);
        let height_f = f64::from(height);
        for (index, value) in values.iter().enumerate() {
            let bar_height = (height_f * value.clamp(0.0, 1.0)).max(1.0);
            let x = layout.start_x + (index as f64 * layout.step);
            let y = height_f - bar_height;
            append_rounded_rect(
                ctx,
                x,
                y,
                layout.scaled_width,
                bar_height,
                bar_corner_radius,
            );
        }

        if ctx.fill().is_err() {
            error!("cavaii: cairo fill failed");
        }
    });

    drawing_area
}

fn build_gpu_widget(config: &AppConfig, bar_values: Rc<RefCell<Vec<f64>>>) -> BarsWidget {
    let bar_width = f64::from(config.visualizer.bar_width.max(1));
    let bar_corner_radius = f64::from(config.visualizer.bar_corner_radius.max(0.0));
    let wave_thickness = f64::from(config.visualizer.wave_thickness.max(1));
    let gap = f64::from(config.visualizer.gap);
    let gradient = resolve_gradient(&config.visualizer);
    let widget = BarsWidget::new(
        bar_values,
        bar_width,
        gap,
        bar_corner_radius,
        wave_thickness,
        config.visualizer.visualizer_type,
        config.visualizer.color_fade,
        config.visualizer.color_orientation,
        gradient,
    );
    widget.set_widget_name("cavaii-bars");
    widget.set_can_target(false);
    widget.set_size_request(
        to_i32(config.overlay.width),
        to_i32(config.overlay.height),
    );
    widget
}

fn attach_frame_tick(
    config: &AppConfig,
    stream: Rc<LiveFrameStream>,
    bar_values: Rc<RefCell<Vec<f64>>>,
    render_target: RenderTarget,
) {
    const BAR_REDRAW_DELTA_THRESHOLD: f64 = 0.003;
    const WAVE_REDRAW_DELTA_THRESHOLD: f64 = 0.0008;
    const WAVE_INTERPOLATION_ALPHA: f64 = 0.4;
    const WAVE_SNAP_EPSILON: f64 = 0.0002;

    let is_wave = config.visualizer.visualizer_type == VisualizerType::Wave;
    let fps = config.visualizer.framerate.max(1);
    let interval_ms = (1000_u64 / u64::from(fps)).max(1);
    let mut last_frame_timestamp = None;
    let mut target_values = Vec::<f64>::new();

    glib::timeout_add_local(Duration::from_millis(interval_ms), move || {
        if !render_target.is_alive() {
            return glib::ControlFlow::Break;
        }

        let frame = stream.latest_frame();
        let frame_timestamp = frame.timestamp_millis;
        let has_new_frame =
            last_frame_timestamp.map_or(true, |timestamp| timestamp != frame_timestamp);
        let mut should_redraw = false;

        if is_wave {
            let mut targets_changed = false;
            if target_values.len() != frame.bars.len() {
                target_values.resize(frame.bars.len(), 0.0);
                targets_changed = true;
            }
            for (slot, value) in target_values.iter_mut().zip(frame.bars.iter()) {
                let next = f64::from(*value);
                if (next - *slot).abs() > WAVE_SNAP_EPSILON {
                    targets_changed = true;
                }
                *slot = next;
            }

            if !target_values.is_empty() {
                let mut displayed = bar_values.borrow_mut();
                if displayed.len() != target_values.len() {
                    displayed.resize(target_values.len(), 0.0);
                }

                let mut max_step = 0.0_f64;
                for (slot, target) in displayed.iter_mut().zip(target_values.iter()) {
                    let delta = *target - *slot;
                    if delta.abs() <= WAVE_SNAP_EPSILON {
                        *slot = *target;
                        continue;
                    }
                    let step = delta * WAVE_INTERPOLATION_ALPHA;
                    *slot += step;
                    if step.abs() > max_step {
                        max_step = step.abs();
                    }
                }
                should_redraw = targets_changed || max_step >= WAVE_REDRAW_DELTA_THRESHOLD;
            }

            if should_redraw {
                render_target.queue();
            }
            return glib::ControlFlow::Continue;
        }

        if has_new_frame {
            last_frame_timestamp = Some(frame_timestamp);
            let mut target = bar_values.borrow_mut();
            let mut force_update = false;
            if target.len() != frame.bars.len() {
                target.resize(frame.bars.len(), 0.0);
                force_update = true;
            }

            let mut max_delta = 0.0_f64;
            for (slot, value) in target.iter().zip(frame.bars.iter()) {
                let next = f64::from(*value);
                let delta = (next - *slot).abs();
                if delta > max_delta {
                    max_delta = delta;
                }
            }

            let should_update = force_update || max_delta >= BAR_REDRAW_DELTA_THRESHOLD;
            if should_update {
                for (slot, value) in target.iter_mut().zip(frame.bars.iter()) {
                    *slot = f64::from(*value);
                }
                should_redraw = frame.peak > 0.001;
            }
        }

        if should_redraw {
            render_target.queue();
        }
        glib::ControlFlow::Continue
    });
}

struct RenderTarget(glib::WeakRef<gtk::Widget>);

impl RenderTarget {
    fn is_alive(&self) -> bool {
        self.0.upgrade().is_some()
    }

    fn queue(&self) {
        if let Some(widget) = self.0.upgrade() {
            widget.queue_draw();
        }
    }
}

#[derive(Default)]
pub(crate) struct BarLayoutCache {
    pub(crate) width: i32,
    pub(crate) bar_count: usize,
    pub(crate) bar_width: f64,
    pub(crate) gap: f64,
    pub(crate) scaled_width: f64,
    pub(crate) scaled_gap: f64,
    pub(crate) start_x: f64,
    pub(crate) step: f64,
}

impl BarLayoutCache {
    fn update(&mut self, width: i32, bar_count: usize, bar_width: f64, gap: f64) {
        if width == self.width
            && bar_count == self.bar_count
            && (bar_width - self.bar_width).abs() < f64::EPSILON
            && (gap - self.gap).abs() < f64::EPSILON
        {
            return;
        }

        self.width = width;
        self.bar_count = bar_count;
        self.bar_width = bar_width;
        self.gap = gap;

        if bar_count == 0 || width <= 0 {
            self.scaled_width = 0.0;
            self.scaled_gap = 0.0;
            self.start_x = 0.0;
            self.step = 0.0;
            return;
        }

        let width_f = f64::from(width);
        let count = bar_count as f64;
        let total_nominal = (count * bar_width) + ((count - 1.0).max(0.0) * gap);
        let scale = if total_nominal > width_f {
            width_f / total_nominal
        } else {
            1.0
        };

        self.scaled_width = (bar_width * scale).max(1.0);
        self.scaled_gap = gap * scale;
        let rendered_total =
            (count * self.scaled_width) + ((count - 1.0).max(0.0) * self.scaled_gap);
        self.start_x = (width_f - rendered_total).max(0.0) * 0.5;
        self.step = self.scaled_width + self.scaled_gap;
    }
}

fn to_i32(value: u32) -> i32 {
    value.max(1).min(i32::MAX as u32) as i32
}

fn append_rounded_rect(
    ctx: &gtk::cairo::Context,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radius: f64,
) {
    if radius <= 0.0 {
        ctx.rectangle(x, y, width, height);
        return;
    }

    let max_radius = (width * 0.5).min(height * 0.5);
    let r = radius.min(max_radius);
    if r <= 0.0 {
        ctx.rectangle(x, y, width, height);
        return;
    }

    let x0 = x;
    let y0 = y;
    let x1 = x + width;
    let y1 = y + height;

    ctx.new_sub_path();
    ctx.move_to(x0 + r, y0);
    ctx.line_to(x1 - r, y0);
    ctx.arc(x1 - r, y0 + r, r, PI * 1.5, PI * 2.0);
    ctx.line_to(x1, y1 - r);
    ctx.arc(x1 - r, y1 - r, r, 0.0, PI * 0.5);
    ctx.line_to(x0 + r, y1);
    ctx.arc(x0 + r, y1 - r, r, PI * 0.5, PI);
    ctx.line_to(x0, y0 + r);
    ctx.arc(x0 + r, y0 + r, r, PI, PI * 1.5);
    ctx.close_path();
}

fn set_paint_source(
    ctx: &gtk::cairo::Context,
    width: i32,
    height: i32,
    gradient: &[RgbaColor],
    orientation: ColorOrientation,
    fade: bool,
) {
    let resolved = if gradient.is_empty() {
        vec![RgbaColor::default()]
    } else {
        gradient.to_vec()
    };

    if resolved.len() == 1 && !fade {
        let color = resolved[0];
        ctx.set_source_rgba(
            f64::from(color.r),
            f64::from(color.g),
            f64::from(color.b),
            f64::from(color.a),
        );
        return;
    }

    let width_f = f64::from(width.max(1));
    let height_f = f64::from(height.max(1));
    let (x0, y0, x1, y1) = gradient_axis(width_f, height_f, orientation);
    let gradient_paint = gtk::cairo::LinearGradient::new(x0, y0, x1, y1);
    for (pos, color) in build_gradient_stops(&resolved, fade) {
        gradient_paint.add_color_stop_rgba(
            pos,
            color.r.into(),
            color.g.into(),
            color.b.into(),
            color.a.into(),
        );
    }
    let _ = ctx.set_source(&gradient_paint);
}

fn gradient_axis(
    width: f64,
    height: f64,
    orientation: ColorOrientation,
) -> (f64, f64, f64, f64) {
    match orientation {
        ColorOrientation::Horizontal => (0.0, 0.0, width.max(1.0), 0.0),
        ColorOrientation::Vertical => (0.0, height.max(1.0), 0.0, 0.0),
    }
}

fn resolve_gradient(visualizer: &VisualizerConfig) -> Vec<RgbaColor> {
    if visualizer.color_gradient.is_empty() {
        VisualizerConfig::default().color_gradient
    } else {
        visualizer.color_gradient.clone()
    }
}

fn build_gradient_stops(gradient: &[RgbaColor], fade: bool) -> Vec<(f64, RgbaColor)> {
    let count = gradient.len().max(1);
    let mut positions: Vec<f64> = (0..count)
        .map(|idx| {
            if count > 1 {
                idx as f64 / (count as f64 - 1.0)
            } else {
                0.0
            }
        })
        .collect();

    positions.push(0.0);
    positions.push(1.0);
    if fade {
        let edge = fade_edge_ratio();
        positions.push(edge);
        positions.push(1.0 - edge);
    }

    positions.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    positions.dedup_by(|a, b| (*a - *b).abs() < 1e-6);

    positions
        .into_iter()
        .map(|pos| {
            let mut color = gradient_color_at(gradient, pos);
            if fade {
                color.a *= fade_factor(pos) as f32;
            }
            (pos, color)
        })
        .collect()
}

fn gradient_color_at(gradient: &[RgbaColor], t: f64) -> RgbaColor {
    if gradient.is_empty() {
        return RgbaColor::default();
    }
    if gradient.len() == 1 {
        return gradient[0];
    }

    let t = t.clamp(0.0, 1.0);
    let scaled = t * (gradient.len() as f64 - 1.0);
    let index = scaled.floor() as usize;
    let next = (index + 1).min(gradient.len() - 1);
    let frac = scaled - index as f64;
    let c0 = gradient[index];
    let c1 = gradient[next];
    RgbaColor {
        r: c0.r + (c1.r - c0.r) * frac as f32,
        g: c0.g + (c1.g - c0.g) * frac as f32,
        b: c0.b + (c1.b - c0.b) * frac as f32,
        a: c0.a + (c1.a - c0.a) * frac as f32,
    }
}

fn fade_edge_ratio() -> f64 {
    0.2_f64.min(0.5)
}

fn fade_factor(position: f64) -> f64 {
    let edge = fade_edge_ratio();
    let left = (position / edge).clamp(0.0, 1.0);
    let right = ((1.0 - position) / edge).clamp(0.0, 1.0);
    left.min(right)
}

pub(super) fn downsample_wave_values<'a>(
    values: &[f64],
    width: i32,
    out: &'a mut Vec<f64>,
) -> &'a [f64] {
    let len = values.len();
    out.clear();
    if len <= 2 || width <= 0 {
        out.extend_from_slice(values);
        return out;
    }

    let width_points = (width as usize / 6).clamp(16, 96);
    let target = len.min(width_points);
    if target >= len {
        out.extend_from_slice(values);
        return out;
    }

    if out.capacity() < target {
        out.reserve(target - out.capacity());
    }
    let bucket_size = len as f64 / target as f64;
    for idx in 0..target {
        let start = (idx as f64 * bucket_size).floor() as usize;
        let end = ((idx as f64 + 1.0) * bucket_size).ceil() as usize;
        let end = end.clamp(start + 1, len);
        let mut sum = 0.0_f64;
        let mut count = 0.0_f64;
        for value in &values[start..end] {
            sum += *value;
            count += 1.0;
        }
        out.push(sum / count.max(1.0));
    }
    out
}

fn draw_wave_line(
    ctx: &gtk::cairo::Context,
    width: i32,
    height: i32,
    values: &[f64],
    line_width: f64,
    points: &mut Vec<(f64, f64)>,
) {
    if values.is_empty() || width <= 0 || height <= 0 {
        return;
    }

    let width_f = f64::from(width);
    let height_f = f64::from(height);
    let mid_y = height_f * 0.5;
    let amplitude = height_f * 0.45;
    let count = values.len();
    let step = if count > 1 {
        width_f / (count as f64 - 1.0)
    } else {
        0.0
    };

    points.clear();
    let points_cap = points.capacity();
    if points_cap < count {
        points.reserve(count - points_cap);
    }
    for (index, value) in values.iter().enumerate() {
        let normalized = (value.clamp(0.0, 1.0) * 2.0) - 1.0;
        let y = mid_y - (normalized * amplitude);
        let x = if count > 1 {
            index as f64 * step
        } else {
            width_f * 0.5
        };
        points.push((x, y));
    }

    ctx.set_line_width(line_width.max(1.0));
    ctx.set_line_cap(gtk::cairo::LineCap::Round);
    ctx.set_line_join(gtk::cairo::LineJoin::Round);
    ctx.new_path();
    ctx.move_to(points[0].0, points[0].1);
    if points.len() == 1 {
        return;
    }
    if points.len() < 4 {
        for point in points.iter().skip(1) {
            ctx.line_to(point.0, point.1);
        }
        return;
    }

    for idx in 0..points.len() - 1 {
        let (p0x, p0y) = if idx == 0 { points[0] } else { points[idx - 1] };
        let (p1x, p1y) = points[idx];
        let (p2x, p2y) = points[idx + 1];
        let (p3x, p3y) = if idx + 2 < points.len() {
            points[idx + 2]
        } else {
            points[idx + 1]
        };
        let c1x = p1x + (p2x - p0x) / 6.0;
        let c1y = p1y + (p2y - p0y) / 6.0;
        let c2x = p2x - (p3x - p1x) / 6.0;
        let c2y = p2y - (p3y - p1y) / 6.0;
        ctx.curve_to(c1x, c1y, c2x, c2y, p2x, p2y);
    }
}
