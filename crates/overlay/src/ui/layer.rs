use gtk::gdk;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use cavaii_common::config::{AppConfig, OverlayConfig, OverlayLayer};
use tracing::warn;

pub fn selected_monitors(_overlay: &OverlayConfig) -> Vec<gdk::Monitor> {
    let Some(display) = gdk::Display::default() else {
        return Vec::new();
    };

    let monitors_model = display.monitors();
    let Some(item) = monitors_model.item(0) else {
        return Vec::new();
    };
    let Ok(monitor) = item.downcast::<gdk::Monitor>() else {
        return Vec::new();
    };

    vec![monitor]
}

pub fn apply_default_size(
    window: &gtk::ApplicationWindow,
    config: &AppConfig,
    _monitor: Option<&gdk::Monitor>,
) {
    let overlay = &config.overlay;
    let width = overlay.width.max(1).min(i32::MAX as u32) as i32;
    let height = overlay.height.max(1).min(i32::MAX as u32) as i32;
    window.set_default_size(width, height);
}

pub fn configure_layer_shell(
    window: &gtk::ApplicationWindow,
    config: &AppConfig,
    monitor: Option<&gdk::Monitor>,
) {
    let overlay = &config.overlay;
    if !gtk4_layer_shell::is_supported() {
        warn!("cavaii: layer-shell is not supported by this compositor/session");
        return;
    }

    window.init_layer_shell();
    window.set_monitor(monitor);
    window.set_namespace(Some("cavaii"));
    window.set_layer(match overlay.layer {
        OverlayLayer::Background => Layer::Background,
        OverlayLayer::Bottom => Layer::Bottom,
        OverlayLayer::Top => Layer::Top,
    });
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_exclusive_zone(0);

    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, false);
        window.set_margin(edge, 0);
    }

    window.set_anchor(Edge::Bottom, true);
    window.set_margin(Edge::Bottom, overlay.anchor_margin.min(i32::MAX as u32) as i32);
}
