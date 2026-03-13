use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::gsk;
use gtk::graphene;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use cavaii_common::config::{ColorOrientation, RgbaColor, VisualizerType};

use super::{BarLayoutCache, downsample_wave_values};

mod imp {
    use super::*;

    pub struct BarsWidget {
        pub(super) bar_values: RefCell<Option<Rc<RefCell<Vec<f64>>>>>,
        pub(super) layout: RefCell<BarLayoutCache>,
        pub(super) wave_samples: RefCell<Vec<f64>>,
        pub(super) wave_points: RefCell<Vec<(f32, f32)>>,
        pub(super) bar_width: RefCell<f64>,
        pub(super) gap: RefCell<f64>,
        pub(super) corner_radius: RefCell<f64>,
        pub(super) wave_thickness: RefCell<f64>,
        pub(super) visualizer_type: RefCell<VisualizerType>,
        pub(super) fade: RefCell<bool>,
        pub(super) orientation: RefCell<ColorOrientation>,
        pub(super) gradient: RefCell<Vec<gdk::RGBA>>,
    }

    impl Default for BarsWidget {
        fn default() -> Self {
            Self {
                bar_values: RefCell::new(None),
                layout: RefCell::new(BarLayoutCache::default()),
                wave_samples: RefCell::new(Vec::new()),
                wave_points: RefCell::new(Vec::new()),
                bar_width: RefCell::new(1.0),
                gap: RefCell::new(0.0),
                corner_radius: RefCell::new(0.0),
                wave_thickness: RefCell::new(1.0),
                visualizer_type: RefCell::new(VisualizerType::Bar),
                fade: RefCell::new(false),
                orientation: RefCell::new(ColorOrientation::Vertical),
                gradient: RefCell::new(vec![gdk::RGBA::new(
                    175.0 / 255.0,
                    198.0 / 255.0,
                    1.0,
                    0.7,
                )]),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BarsWidget {
        const NAME: &'static str = "CavaiiGpuBarsWidget";
        type Type = super::BarsWidget;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for BarsWidget {}

    impl WidgetImpl for BarsWidget {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(values) = self.bar_values.borrow().as_ref().cloned() else {
                return;
            };
            let values = values.borrow();
            let widget = self.obj();
            let width = widget.width();
            let height = widget.height();
            if values.is_empty() || width <= 0 || height <= 0 {
                return;
            }

            let bar_width = *self.bar_width.borrow();
            let gap = *self.gap.borrow();
            let corner_radius = *self.corner_radius.borrow();
            let wave_thickness = *self.wave_thickness.borrow();
            let visualizer_type = *self.visualizer_type.borrow();
            let fade = *self.fade.borrow();
            let orientation = *self.orientation.borrow();
            let mut layout = self.layout.borrow_mut();
            let gradient = self.gradient.borrow();
            let height_f = height as f32;

            if visualizer_type == VisualizerType::Wave {
                let width_f = width as f32;
                let height_f = height as f32;
                let mid_y = height_f * 0.5;
                let amplitude = height_f * 0.45;
                let mut samples_scratch = self.wave_samples.borrow_mut();
                let sampled = downsample_wave_values(&values, width, &mut samples_scratch);
                let count = sampled.len();
                let step = if count > 1 {
                    width_f / (count as f32 - 1.0)
                } else {
                    0.0
                };

                let mut points = self.wave_points.borrow_mut();
                points.clear();
                let points_cap = points.capacity();
                if points_cap < count {
                    points.reserve(count - points_cap);
                }
                for (index, value) in sampled.iter().enumerate() {
                    let normalized = (value.clamp(0.0, 1.0) * 2.0) - 1.0;
                    let y = mid_y - (normalized as f32 * amplitude);
                    let x = if count > 1 {
                        index as f32 * step
                    } else {
                        width_f * 0.5
                    };
                    points.push((x, y));
                }

                let path_builder = gsk::PathBuilder::new();
                path_builder.move_to(points[0].0, points[0].1);
                if points.len() > 1 && points.len() < 4 {
                    for point in points.iter().skip(1) {
                        path_builder.line_to(point.0, point.1);
                    }
                } else if points.len() > 1 {
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
                        path_builder.cubic_to(c1x, c1y, c2x, c2y, p2x, p2y);
                    }
                }

                let path = path_builder.to_path();
                let stroke = gsk::Stroke::builder(wave_thickness.max(1.0) as f32)
                    .line_cap(gsk::LineCap::Round)
                    .line_join(gsk::LineJoin::Round)
                    .build();
                let bounds = graphene::Rect::new(0.0, 0.0, width_f, height_f);
                let stroke_node = if fade || gradient.len() > 1 {
                    let stops = build_gradient_stops(&gradient, fade);
                    let (sx, sy, ex, ey) = gradient_axis(width_f, height_f, orientation);
                    let start = graphene::Point::new(sx, sy);
                    let end = graphene::Point::new(ex, ey);
                    let gradient = gsk::LinearGradientNode::new(&bounds, &start, &end, &stops);
                    gsk::StrokeNode::new(&gradient, &path, &stroke)
                } else {
                    let color = gradient.first().cloned().unwrap_or_else(|| {
                        gdk::RGBA::new(175.0 / 255.0, 198.0 / 255.0, 1.0, 0.7)
                    });
                    let color_node = gsk::ColorNode::new(&color, &bounds);
                    gsk::StrokeNode::new(&color_node, &path, &stroke)
                };
                snapshot.append_node(&stroke_node);
                return;
            }

            layout.update(width, values.len(), bar_width, gap);
            for (index, value) in values.iter().enumerate() {
                let bar_height = (height_f * value.clamp(0.0, 1.0) as f32).max(1.0);
                let x = layout.start_x + (index as f64 * layout.step);
                let y = (height as f64) - bar_height as f64;
                let rect = graphene::Rect::new(
                    x as f32,
                    y as f32,
                    layout.scaled_width as f32,
                    bar_height,
                );
                let center_x = x + (layout.scaled_width * 0.5);
                let center_y = y + (bar_height as f64 * 0.5);
                let gradient_t = gradient_position(center_x, center_y, width as f64, height as f64, orientation);
                let base_color = gradient_color_at(&gradient, gradient_t);
                let alpha = if fade {
                    let factor = edge_fade_factor(center_x, width as f64);
                    (base_color.alpha() as f64 * factor) as f32
                } else {
                    base_color.alpha()
                };
                let bar_color = gdk::RGBA::new(
                    base_color.red(),
                    base_color.green(),
                    base_color.blue(),
                    alpha,
                );
                if corner_radius > 0.0 {
                    let radius = corner_radius
                        .min(layout.scaled_width * 0.5)
                        .min(bar_height as f64 * 0.5) as f32;
                    let corner = graphene::Size::new(radius, radius);
                    let rounded = gsk::RoundedRect::new(rect, corner, corner, corner, corner);
                    snapshot.push_rounded_clip(&rounded);
                    snapshot.append_color(&bar_color, &rect);
                    snapshot.pop();
                } else {
                    snapshot.append_color(&bar_color, &rect);
                }
            }
        }
    }
}

glib::wrapper! {
    pub struct BarsWidget(ObjectSubclass<imp::BarsWidget>)
        @extends gtk::Widget, gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl BarsWidget {
    pub fn new(
        bar_values: Rc<RefCell<Vec<f64>>>,
        bar_width: f64,
        gap: f64,
        corner_radius: f64,
        wave_thickness: f64,
        visualizer_type: VisualizerType,
        fade: bool,
        orientation: ColorOrientation,
        gradient: Vec<RgbaColor>,
    ) -> Self {
        let widget: Self = glib::Object::new();
        let imp = imp::BarsWidget::from_obj(&widget);
        *imp.bar_values.borrow_mut() = Some(bar_values);
        *imp.bar_width.borrow_mut() = bar_width;
        *imp.gap.borrow_mut() = gap;
        *imp.corner_radius.borrow_mut() = corner_radius;
        *imp.wave_thickness.borrow_mut() = wave_thickness;
        *imp.visualizer_type.borrow_mut() = visualizer_type;
        *imp.fade.borrow_mut() = fade;
        *imp.orientation.borrow_mut() = orientation;
        *imp.gradient.borrow_mut() = if gradient.is_empty() {
            vec![gdk::RGBA::new(175.0 / 255.0, 198.0 / 255.0, 1.0, 0.7)]
        } else {
            gradient
                .into_iter()
                .map(|color| gdk::RGBA::new(color.r, color.g, color.b, color.a))
                .collect()
        };
        widget
    }
}

fn edge_fade_factor(x: f64, width: f64) -> f64 {
    let width = width.max(1.0);
    let position = (x / width).clamp(0.0, 1.0);
    fade_factor(position)
}

fn build_gradient_stops(gradient: &[gdk::RGBA], fade: bool) -> Vec<gsk::ColorStop> {
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
                let alpha = color.alpha() as f64 * fade_factor(pos);
                color.set_alpha(alpha as f32);
            }
            gsk::ColorStop::new(pos as f32, color)
        })
        .collect()
}

fn gradient_color_at(gradient: &[gdk::RGBA], t: f64) -> gdk::RGBA {
    if gradient.is_empty() {
        return gdk::RGBA::new(175.0 / 255.0, 198.0 / 255.0, 1.0, 0.7);
    }
    if gradient.len() == 1 {
        return gradient[0].clone();
    }

    let t = t.clamp(0.0, 1.0);
    let scaled = t * (gradient.len() as f64 - 1.0);
    let index = scaled.floor() as usize;
    let next = (index + 1).min(gradient.len() - 1);
    let frac = scaled - index as f64;
    let c0 = &gradient[index];
    let c1 = &gradient[next];
    gdk::RGBA::new(
        c0.red() + (c1.red() - c0.red()) * frac as f32,
        c0.green() + (c1.green() - c0.green()) * frac as f32,
        c0.blue() + (c1.blue() - c0.blue()) * frac as f32,
        c0.alpha() + (c1.alpha() - c0.alpha()) * frac as f32,
    )
}

fn gradient_position(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    orientation: ColorOrientation,
) -> f64 {
    match orientation {
        ColorOrientation::Horizontal => (x / width.max(1.0)).clamp(0.0, 1.0),
        ColorOrientation::Vertical => ((height.max(1.0) - y) / height.max(1.0)).clamp(0.0, 1.0),
    }
}

fn gradient_axis(
    width: f32,
    height: f32,
    orientation: ColorOrientation,
) -> (f32, f32, f32, f32) {
    match orientation {
        ColorOrientation::Horizontal => (0.0, 0.0, width.max(1.0), 0.0),
        ColorOrientation::Vertical => (0.0, height.max(1.0), 0.0, 0.0),
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
