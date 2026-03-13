use std::env;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayLayer {
    Background,
    Bottom,
    Top,
}

impl OverlayLayer {
    fn parse(value: &str) -> Result<Self, ConfigLoadError> {
        match value {
            "background" => Ok(Self::Background),
            "bottom" => Ok(Self::Bottom),
            "top" => Ok(Self::Top),
            _ => Err(ConfigLoadError::Parse(format!(
                "unknown overlay.layer value: {value}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayConfig {
    pub layer: OverlayLayer,
    pub anchor_margin: u32,
    pub width: u32,
    pub height: u32,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            layer: OverlayLayer::Background,
            anchor_margin: 10,
            width: 1000,
            height: 300,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisualizerBackend {
    Auto,
    Pipewire,
    Cava,
    Dummy,
}

impl VisualizerBackend {
    fn parse(value: &str) -> Result<Self, ConfigLoadError> {
        match value {
            "auto" => Ok(Self::Auto),
            "pipewire" => Ok(Self::Pipewire),
            "cava" => Ok(Self::Cava),
            "dummy" => Ok(Self::Dummy),
            _ => Err(ConfigLoadError::Parse(format!(
                "unknown visualizer.backend value: {value}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RgbaColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl RgbaColor {
    fn parse(value: &str) -> Result<Self, ConfigLoadError> {
        let normalized = value.trim();
        if let Some(hex) = normalized.strip_prefix('#') {
            return parse_hex_color(hex);
        }

        let (parts, has_alpha) = if let Some(inner) = normalized.strip_prefix("rgba(") {
            (
                inner
                    .strip_suffix(')')
                    .unwrap_or(inner)
                    .split(',')
                    .map(str::trim)
                    .collect::<Vec<_>>(),
                true,
            )
        } else if let Some(inner) = normalized.strip_prefix("rgb(") {
            (
                inner
                    .strip_suffix(')')
                    .unwrap_or(inner)
                    .split(',')
                    .map(str::trim)
                    .collect::<Vec<_>>(),
                false,
            )
        } else {
            let parsed = normalized.split(',').map(str::trim).collect::<Vec<_>>();
            let has_alpha = parsed.len() == 4;
            (parsed, has_alpha)
        };

        let expected_len = if has_alpha { 4 } else { 3 };
        if parts.len() != expected_len {
            return Err(ConfigLoadError::Parse(format!(
                "invalid color value: {value}"
            )));
        }

        let mut r = parse_f32("color.r", parts[0])?;
        let mut g = parse_f32("color.g", parts[1])?;
        let mut b = parse_f32("color.b", parts[2])?;
        let a = if has_alpha {
            parse_f32("color.a", parts[3])?.clamp(0.0, 1.0)
        } else {
            1.0
        };

        if r > 1.0 || g > 1.0 || b > 1.0 {
            r = (r / 255.0).clamp(0.0, 1.0);
            g = (g / 255.0).clamp(0.0, 1.0);
            b = (b / 255.0).clamp(0.0, 1.0);
        } else {
            r = r.clamp(0.0, 1.0);
            g = g.clamp(0.0, 1.0);
            b = b.clamp(0.0, 1.0);
        }

        Ok(Self { r, g, b, a })
    }
}

fn parse_hex_color(hex: &str) -> Result<RgbaColor, ConfigLoadError> {
    let parse_chan = |idx: usize| -> Result<u8, ConfigLoadError> {
        u8::from_str_radix(&hex[idx..idx + 2], 16)
            .map_err(|_| ConfigLoadError::Parse(format!("invalid hex color: #{hex}")))
    };
    match hex.len() {
        6 => {
            let r = parse_chan(0)? as f32 / 255.0;
            let g = parse_chan(2)? as f32 / 255.0;
            let b = parse_chan(4)? as f32 / 255.0;
            Ok(RgbaColor { r, g, b, a: 1.0 })
        }
        8 => {
            let r = parse_chan(0)? as f32 / 255.0;
            let g = parse_chan(2)? as f32 / 255.0;
            let b = parse_chan(4)? as f32 / 255.0;
            let a = parse_chan(6)? as f32 / 255.0;
            Ok(RgbaColor { r, g, b, a })
        }
        _ => Err(ConfigLoadError::Parse(format!(
            "invalid hex color length: #{hex}"
        ))),
    }
}

impl Default for RgbaColor {
    fn default() -> Self {
        Self {
            r: 0.12,
            g: 0.88,
            b: 0.68,
            a: 0.9,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualizerType {
    Bar,
    Wave,
}

impl VisualizerType {
    fn parse(value: &str) -> Result<Self, ConfigLoadError> {
        match value {
            "bar" => Ok(Self::Bar),
            "wave" => Ok(Self::Wave),
            _ => Err(ConfigLoadError::Parse(format!(
                "unknown visualizer.type value: {value}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorOrientation {
    Vertical,
    Horizontal,
}

impl ColorOrientation {
    fn parse(value: &str) -> Result<Self, ConfigLoadError> {
        match value {
            "vertical" => Ok(Self::Vertical),
            "horizontal" => Ok(Self::Horizontal),
            _ => Err(ConfigLoadError::Parse(format!(
                "unknown color.orientation value: {value}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VisualizerConfig {
    pub backend: VisualizerBackend,
    pub visualizer_type: VisualizerType,
    pub bars: usize,
    pub bar_width: u32,
    pub bar_corner_radius: f32,
    pub wave_thickness: u32,
    pub gap: u32,
    pub framerate: u32,
    pub color_gradient: Vec<RgbaColor>,
    pub color_orientation: ColorOrientation,
    pub color_fade: bool,
    pub gpu: bool,
    pub pipewire_attack: f32,
    pub pipewire_decay: f32,
    pub pipewire_gain: f32,
    pub pipewire_curve: f32,
    pub pipewire_neighbor_mix: f32,
}

impl Default for VisualizerConfig {
    fn default() -> Self {
        Self {
            backend: VisualizerBackend::Pipewire,
            visualizer_type: VisualizerType::Bar,
            bars: 120,
            bar_width: 12,
            bar_corner_radius: 20.0,
            wave_thickness: 2,
            gap: 5,
            framerate: 60,
            color_gradient: vec![RgbaColor {
                r: 175.0 / 255.0,
                g: 198.0 / 255.0,
                b: 1.0,
                a: 0.7,
            }],
            color_orientation: ColorOrientation::Vertical,
            color_fade: true,
            gpu: true,
            pipewire_attack: 0.14,
            pipewire_decay: 0.975,
            pipewire_gain: 1.20,
            pipewire_curve: 0.95,
            pipewire_neighbor_mix: 0.24,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AppConfig {
    pub overlay: OverlayConfig,
    pub visualizer: VisualizerConfig,
    pub daemon: DaemonConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DaemonConfig {
    pub enabled: bool,
    pub poll_interval_ms: u64,
    pub activity_threshold: f32,
    pub activate_delay_ms: u64,
    pub deactivate_delay_ms: u64,
    pub stop_on_silence: bool,
    pub notify_on_error: bool,
    pub notify_cooldown_seconds: u64,
    pub allowed_processes: Vec<String>,
    pub overlay_command: String,
    pub overlay_args: Vec<String>,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            poll_interval_ms: 90,
            activity_threshold: 0.035,
            activate_delay_ms: 180,
            deactivate_delay_ms: 2200,
            stop_on_silence: true,
            notify_on_error: true,
            notify_cooldown_seconds: 45,
            allowed_processes: Vec::new(),
            overlay_command: "cavaii".to_owned(),
            overlay_args: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct VisualizerColorOverrides {
    pub gradient: Option<Vec<RgbaColor>>,
    pub orientation: Option<ColorOrientation>,
    pub fade: Option<bool>,
}

#[derive(Debug)]
pub enum ConfigLoadError {
    Io(std::io::Error),
    Parse(String),
}

impl Display for ConfigLoadError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::Parse(msg) => write!(f, "config parse error: {msg}"),
        }
    }
}

impl Error for ConfigLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Parse(_) => None,
        }
    }
}

pub fn default_config_path() -> PathBuf {
    if let Ok(override_path) = env::var("CAVAII_CONFIG") {
        return PathBuf::from(override_path);
    }

    if let Ok(config_home) = env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(config_home).join("cavaii/config.toml");
    }

    if let Ok(home) = env::var("HOME") {
        return PathBuf::from(home).join(".config/cavaii/config.toml");
    }

    PathBuf::from("cavaii.toml")
}

pub fn default_colors_path(config_path: &Path) -> PathBuf {
    match config_path.parent() {
        Some(parent) => parent.join("colors.toml"),
        None => PathBuf::from("colors.toml"),
    }
}

pub fn load_or_default(path: &Path) -> Result<AppConfig, ConfigLoadError> {
    let raw = match fs::read_to_string(path) {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(AppConfig::default()),
        Err(err) => return Err(ConfigLoadError::Io(err)),
    };

    parse_config(&raw)
}

pub fn load_color_overrides(path: &Path) -> Result<VisualizerColorOverrides, ConfigLoadError> {
    let raw = match fs::read_to_string(path) {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(VisualizerColorOverrides::default());
        }
        Err(err) => return Err(ConfigLoadError::Io(err)),
    };

    parse_color_overrides(&raw)
}

pub fn apply_color_overrides(config: &mut AppConfig, overrides: VisualizerColorOverrides) {
    if let Some(orientation) = overrides.orientation {
        config.visualizer.color_orientation = orientation;
    }
    if let Some(fade) = overrides.fade {
        config.visualizer.color_fade = fade;
    }
    if let Some(gradient) = overrides.gradient {
        if !gradient.is_empty() {
            config.visualizer.color_gradient = gradient;
        }
    }
}

fn parse_config(raw: &str) -> Result<AppConfig, ConfigLoadError> {
    let mut config = AppConfig::default();
    let mut section: Option<&str> = None;
    let lines = raw.lines().collect::<Vec<_>>();
    let mut line_idx = 0usize;

    while line_idx < lines.len() {
        let line_no = line_idx + 1;
        let trimmed = lines[line_idx].trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            line_idx += 1;
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let next = &trimmed[1..trimmed.len() - 1];
            section = Some(next);
            line_idx += 1;
            continue;
        }

        let Some((key, value_raw)) = trimmed.split_once('=') else {
            return Err(ConfigLoadError::Parse(format!(
                "line {line_no}: invalid key/value line: {trimmed}"
            )));
        };

        let key = key.trim();
        let mut value_owned = value_raw.to_owned();
        if value_needs_multiline_array(value_raw) {
            let mut depth = array_bracket_delta(value_raw);
            while depth > 0 {
                line_idx += 1;
                if line_idx >= lines.len() {
                    return Err(ConfigLoadError::Parse(format!(
                        "line {line_no}: unterminated array value for key {key}"
                    )));
                }
                value_owned.push('\n');
                value_owned.push_str(lines[line_idx]);
                depth += array_bracket_delta(lines[line_idx]);
            }
        }
        let value = normalize_value(&value_owned);

        match section {
            Some("overlay") => parse_overlay_key(&mut config.overlay, key, &value)
                .map_err(|err| with_line_context(err, line_no))?,
            Some("visualizer") => parse_visualizer_key(&mut config.visualizer, key, &value)
                .map_err(|err| with_line_context(err, line_no))?,
            Some("daemon") => parse_daemon_key(&mut config.daemon, key, &value)
                .map_err(|err| with_line_context(err, line_no))?,
            Some(other) => {
                return Err(ConfigLoadError::Parse(format!(
                    "line {line_no}: unknown section [{other}]"
                )));
            }
            None => {
                return Err(ConfigLoadError::Parse(format!(
                    "line {line_no}: key/value before a section header"
                )));
            }
        }
        line_idx += 1;
    }

    Ok(config)
}

fn parse_color_overrides(raw: &str) -> Result<VisualizerColorOverrides, ConfigLoadError> {
    let mut overrides = VisualizerColorOverrides::default();
    let mut section: Option<&str> = None;
    let lines = raw.lines().collect::<Vec<_>>();
    let mut line_idx = 0usize;

    while line_idx < lines.len() {
        let line_no = line_idx + 1;
        let trimmed = lines[line_idx].trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            line_idx += 1;
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let next = &trimmed[1..trimmed.len() - 1];
            if next != "color" {
                return Err(ConfigLoadError::Parse(format!(
                    "line {line_no}: unknown section [{next}]"
                )));
            }
            section = Some(next);
            line_idx += 1;
            continue;
        }

        let Some((key, value_raw)) = trimmed.split_once('=') else {
            line_idx += 1;
            continue;
        };

        if section.is_none() {
            line_idx += 1;
            continue;
        }

        let key = key.trim();
        let mut value_owned = value_raw.to_owned();
        if value_needs_multiline_array(value_raw) {
            let mut depth = array_bracket_delta(value_raw);
            while depth > 0 {
                line_idx += 1;
                if line_idx >= lines.len() {
                    return Err(ConfigLoadError::Parse(format!(
                        "line {line_no}: unterminated array value for key {key}"
                    )));
                }
                value_owned.push('\n');
                value_owned.push_str(lines[line_idx]);
                depth += array_bracket_delta(lines[line_idx]);
            }
        }
        let value = normalize_value(&value_owned);

        match key {
            "gradient" => {
                let gradient =
                    parse_gradient(&value).map_err(|err| with_line_context(err, line_no))?;
                overrides.gradient = Some(gradient);
            }
            "orientation" => {
                let orientation =
                    ColorOrientation::parse(&value).map_err(|err| with_line_context(err, line_no))?;
                overrides.orientation = Some(orientation);
            }
            "fade" => {
                let fade =
                    parse_bool(key, &value).map_err(|err| with_line_context(err, line_no))?;
                overrides.fade = Some(fade);
            }
            _ => {
                return Err(ConfigLoadError::Parse(format!(
                    "line {line_no}: unknown color key: {key}"
                )));
            }
        }
        line_idx += 1;
    }

    if let Some(existing) = overrides.gradient.take() {
        overrides.gradient = Some(validate_gradient(existing)?);
    }

    Ok(overrides)
}

fn parse_overlay_key(
    overlay: &mut OverlayConfig,
    key: &str,
    value: &str,
) -> Result<(), ConfigLoadError> {
    match key {
        "layer" => overlay.layer = OverlayLayer::parse(value)?,
        "anchor_margin" => overlay.anchor_margin = parse_u32(key, value)?,
        "width" => overlay.width = parse_u32(key, value)?,
        "height" => overlay.height = parse_u32(key, value)?,
        _ => {
            return Err(ConfigLoadError::Parse(format!(
                "unknown overlay key: {key}"
            )));
        }
    }
    Ok(())
}

fn parse_visualizer_key(
    visualizer: &mut VisualizerConfig,
    key: &str,
    value: &str,
) -> Result<(), ConfigLoadError> {
    match key {
        "backend" => visualizer.backend = VisualizerBackend::parse(value)?,
        "type" => visualizer.visualizer_type = VisualizerType::parse(value)?,
        "bars" => visualizer.bars = parse_usize(key, value)?,
        "bar_width" => visualizer.bar_width = parse_u32(key, value)?,
        "bar_corner_radius" => {
            visualizer.bar_corner_radius = parse_f32(key, value)?.max(0.0);
        }
        "wave_thickness" => visualizer.wave_thickness = parse_u32(key, value)?.max(1),
        "gap" => visualizer.gap = parse_u32(key, value)?,
        "framerate" => visualizer.framerate = parse_u32(key, value)?,
        "gpu" => visualizer.gpu = parse_bool(key, value)?,
        "pipewire_attack" => visualizer.pipewire_attack = parse_f32(key, value)?,
        "pipewire_decay" => visualizer.pipewire_decay = parse_f32(key, value)?,
        "pipewire_gain" => visualizer.pipewire_gain = parse_f32(key, value)?,
        "pipewire_curve" => visualizer.pipewire_curve = parse_f32(key, value)?,
        "pipewire_neighbor_mix" => visualizer.pipewire_neighbor_mix = parse_f32(key, value)?,
        _ => {
            return Err(ConfigLoadError::Parse(format!(
                "unknown visualizer key: {key}"
            )));
        }
    }
    Ok(())
}

fn parse_daemon_key(
    daemon: &mut DaemonConfig,
    key: &str,
    value: &str,
) -> Result<(), ConfigLoadError> {
    match key {
        "enabled" => daemon.enabled = parse_bool(key, value)?,
        "poll_interval_ms" => daemon.poll_interval_ms = parse_u64(key, value)?.max(16),
        "activity_threshold" => daemon.activity_threshold = parse_f32(key, value)?.clamp(0.0, 1.0),
        "activate_delay_ms" => daemon.activate_delay_ms = parse_u64(key, value)?,
        "deactivate_delay_ms" => daemon.deactivate_delay_ms = parse_u64(key, value)?,
        "stop_on_silence" => daemon.stop_on_silence = parse_bool(key, value)?,
        "notify_on_error" => daemon.notify_on_error = parse_bool(key, value)?,
        "notify_cooldown_seconds" => daemon.notify_cooldown_seconds = parse_u64(key, value)?,
        "allowed_processes" => daemon.allowed_processes = parse_string_list(value),
        "overlay_command" => {
            let command = parse_optional_string(value).unwrap_or_default();
            daemon.overlay_command = if command.is_empty() {
                DaemonConfig::default().overlay_command
            } else {
                command
            };
        }
        "overlay_args" => daemon.overlay_args = parse_string_list(value),
        _ => {
            return Err(ConfigLoadError::Parse(format!("unknown daemon key: {key}")));
        }
    }
    Ok(())
}

fn parse_u32(key: &str, value: &str) -> Result<u32, ConfigLoadError> {
    value
        .parse::<u32>()
        .map_err(|_| ConfigLoadError::Parse(format!("invalid u32 for {key}: {value}")))
}

fn parse_usize(key: &str, value: &str) -> Result<usize, ConfigLoadError> {
    value
        .parse::<usize>()
        .map_err(|_| ConfigLoadError::Parse(format!("invalid usize for {key}: {value}")))
}

fn parse_u64(key: &str, value: &str) -> Result<u64, ConfigLoadError> {
    value
        .parse::<u64>()
        .map_err(|_| ConfigLoadError::Parse(format!("invalid u64 for {key}: {value}")))
}

fn parse_f32(key: &str, value: &str) -> Result<f32, ConfigLoadError> {
    value
        .parse::<f32>()
        .map_err(|_| ConfigLoadError::Parse(format!("invalid f32 for {key}: {value}")))
}

fn parse_bool(key: &str, value: &str) -> Result<bool, ConfigLoadError> {
    match value {
        "true" | "1" | "enabled" => Ok(true),
        "false" | "0" | "disabled" => Ok(false),
        _ => Err(ConfigLoadError::Parse(format!(
            "invalid bool for {key}: {value}"
        ))),
    }
}

fn with_line_context(error: ConfigLoadError, line_no: usize) -> ConfigLoadError {
    match error {
        ConfigLoadError::Parse(message) => {
            ConfigLoadError::Parse(format!("line {line_no}: {message}"))
        }
        other => other,
    }
}

fn parse_optional_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn parse_string_list(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let inner = if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.len() >= 2 {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };

    inner
        .split(',')
        .map(str::trim)
        .map(|item| item.trim_matches('"').trim_matches('\'').trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

fn value_needs_multiline_array(value: &str) -> bool {
    array_bracket_delta(value) > 0
}

fn array_bracket_delta(value: &str) -> i32 {
    let mut in_quotes = false;
    let mut quote_char = '\0';
    let mut escaped = false;
    let mut depth = 0_i32;

    for ch in value.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' || ch == '\'' {
            if in_quotes && ch == quote_char {
                in_quotes = false;
                quote_char = '\0';
            } else if !in_quotes {
                in_quotes = true;
                quote_char = ch;
            }
            continue;
        }
        if !in_quotes {
            if ch == '[' {
                depth += 1;
            } else if ch == ']' {
                depth -= 1;
            }
        }
    }
    depth
}

fn validate_gradient(mut gradient: Vec<RgbaColor>) -> Result<Vec<RgbaColor>, ConfigLoadError> {
    if gradient.is_empty() {
        return Err(ConfigLoadError::Parse(
            "gradient must include at least 1 color".to_owned(),
        ));
    }
    if gradient.len() > 5 {
        gradient.truncate(5);
    }
    Ok(gradient)
}

fn parse_gradient(value: &str) -> Result<Vec<RgbaColor>, ConfigLoadError> {
    let gradient = parse_rgba_list(value)?;
    validate_gradient(gradient)
}

fn parse_rgba_list(value: &str) -> Result<Vec<RgbaColor>, ConfigLoadError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let inner = if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.len() >= 2 {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };

    let mut items = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut quote_char = '\0';
    let mut escaped = false;
    let mut paren_depth = 0_u32;

    for ch in inner.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' || ch == '\'' {
            if in_quotes && ch == quote_char {
                in_quotes = false;
                quote_char = '\0';
            } else if !in_quotes {
                in_quotes = true;
                quote_char = ch;
            }
            continue;
        }
        if !in_quotes {
            if ch == '(' {
                paren_depth = paren_depth.saturating_add(1);
            } else if ch == ')' {
                paren_depth = paren_depth.saturating_sub(1);
            }
        }
        if ch == ',' && !in_quotes && paren_depth == 0 {
            let item = current.trim();
            if !item.is_empty() {
                items.push(item.to_owned());
            }
            current.clear();
            continue;
        }
        current.push(ch);
    }

    let item = current.trim();
    if !item.is_empty() {
        items.push(item.to_owned());
    }

    let mut colors = Vec::new();
    for item in items {
        colors.push(RgbaColor::parse(&item)?);
    }
    Ok(colors)
}

fn normalize_value(raw: &str) -> String {
    let mut without_comment = String::new();
    let mut in_quotes = false;
    let mut escaped = false;

    for ch in raw.chars() {
        if ch == '"' && !escaped {
            in_quotes = !in_quotes;
            without_comment.push(ch);
            continue;
        }
        if ch == '#' && !in_quotes {
            break;
        }
        escaped = ch == '\\' && !escaped;
        without_comment.push(ch);
    }

    let mut cleaned = without_comment.trim().trim_end_matches([',', ';']).trim();

    if cleaned.len() >= 2 {
        let quoted_double = cleaned.starts_with('"') && cleaned.ends_with('"');
        let quoted_single = cleaned.starts_with('\'') && cleaned.ends_with('\'');
        if quoted_double || quoted_single {
            cleaned = &cleaned[1..cleaned.len() - 1];
        }
    }

    cleaned.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::{
        AppConfig, ColorOrientation, DaemonConfig, OverlayLayer, VisualizerBackend,
        VisualizerType, apply_color_overrides, parse_color_overrides, parse_config,
    };

    #[test]
    fn parses_valid_config() {
        let raw = r#"
        [overlay]
        layer = "top"
        anchor_margin = 12
        width = 1200
        height = 140

        [visualizer]
        backend = "dummy"
        type = "wave"
        bars = 64
        bar_width = 5
        bar_corner_radius = 6
        wave_thickness = 4
        gap = 2
        framerate = 75
        gpu = "disabled"
        pipewire_attack = 0.2
        pipewire_decay = 0.9
        pipewire_gain = 1.5
        pipewire_curve = 0.8
        pipewire_neighbor_mix = 0.3

        [daemon]
        enabled = true
        poll_interval_ms = 50
        activity_threshold = 0.045
        activate_delay_ms = 120
        deactivate_delay_ms = 1800
        stop_on_silence = false
        notify_on_error = true
        notify_cooldown_seconds = 30
        allowed_processes = ["spotify", "vlc"]
        overlay_command = "cargo"
        overlay_args = ["run", "-p", "cavaii"]
        "#;

        let parsed = match parse_config(raw) {
            Ok(value) => value,
            Err(err) => panic!("valid config should parse, got error: {err}"),
        };
        assert_eq!(parsed.overlay.layer, OverlayLayer::Top);
        assert_eq!(parsed.overlay.anchor_margin, 12);
        assert_eq!(parsed.overlay.width, 1200);
        assert_eq!(parsed.overlay.height, 140);
        assert_eq!(parsed.visualizer.backend, VisualizerBackend::Dummy);
        assert_eq!(parsed.visualizer.visualizer_type, VisualizerType::Wave);
        assert_eq!(parsed.visualizer.bars, 64);
        assert_eq!(parsed.visualizer.bar_width, 5);
        assert!((parsed.visualizer.bar_corner_radius - 6.0).abs() < 1e-5);
        assert_eq!(parsed.visualizer.wave_thickness, 4);
        assert_eq!(parsed.visualizer.color_orientation, ColorOrientation::Vertical);
        assert!(parsed.visualizer.color_fade);
        assert_eq!(parsed.visualizer.gap, 2);
        assert_eq!(parsed.visualizer.framerate, 75);
        assert_eq!(parsed.visualizer.color_gradient.len(), 1);
        assert!((parsed.visualizer.color_gradient[0].r - (175.0 / 255.0)).abs() < 1e-5);
        assert!(!parsed.visualizer.gpu);
        assert_eq!(parsed.visualizer.pipewire_attack, 0.2);
        assert_eq!(parsed.visualizer.pipewire_decay, 0.9);
        assert_eq!(parsed.visualizer.pipewire_gain, 1.5);
        assert_eq!(parsed.visualizer.pipewire_curve, 0.8);
        assert_eq!(parsed.visualizer.pipewire_neighbor_mix, 0.3);
        assert_eq!(
            parsed.daemon,
            DaemonConfig {
                enabled: true,
                poll_interval_ms: 50,
                activity_threshold: 0.045,
                activate_delay_ms: 120,
                deactivate_delay_ms: 1800,
                stop_on_silence: false,
                notify_on_error: true,
                notify_cooldown_seconds: 30,
                allowed_processes: vec!["spotify".to_owned(), "vlc".to_owned()],
                overlay_command: "cargo".to_owned(),
                overlay_args: vec![
                    "run".to_owned(),
                    "-p".to_owned(),
                    "cavaii".to_owned()
                ],
            }
        );
    }

    #[test]
    fn returns_default_for_empty_config() {
        let parsed = match parse_config("") {
            Ok(value) => value,
            Err(err) => panic!("empty config should parse, got error: {err}"),
        };
        assert_eq!(parsed, AppConfig::default());
    }

    #[test]
    fn built_in_defaults_match_expected_no_config_setup() {
        let config = AppConfig::default();

        assert_eq!(config.overlay.layer, OverlayLayer::Background);
        assert_eq!(config.overlay.anchor_margin, 10);
        assert_eq!(config.overlay.width, 1000);
        assert_eq!(config.overlay.height, 300);

        assert_eq!(config.visualizer.backend, VisualizerBackend::Pipewire);
        assert_eq!(config.visualizer.visualizer_type, VisualizerType::Bar);
        assert_eq!(config.visualizer.bars, 120);
        assert_eq!(config.visualizer.bar_width, 12);
        assert!((config.visualizer.bar_corner_radius - 20.0).abs() < 1e-5);
        assert_eq!(config.visualizer.wave_thickness, 2);
        assert!(config.visualizer.color_fade);
        assert_eq!(config.visualizer.gap, 5);
        assert_eq!(config.visualizer.framerate, 60);
        assert_eq!(config.visualizer.color_gradient.len(), 1);
        assert!((config.visualizer.color_gradient[0].r - (175.0 / 255.0)).abs() < 1e-5);
        assert!((config.visualizer.color_gradient[0].g - (198.0 / 255.0)).abs() < 1e-5);
        assert!((config.visualizer.color_gradient[0].b - 1.0).abs() < 1e-5);
        assert!((config.visualizer.color_gradient[0].a - 0.7).abs() < 1e-5);
        assert_eq!(config.visualizer.color_orientation, ColorOrientation::Vertical);
        assert!(config.visualizer.gpu);

        assert!(config.daemon.enabled);
        assert_eq!(config.daemon.poll_interval_ms, 90);
        assert!((config.daemon.activity_threshold - 0.035).abs() < 1e-5);
        assert_eq!(config.daemon.activate_delay_ms, 180);
        assert_eq!(config.daemon.deactivate_delay_ms, 2200);
        assert!(config.daemon.stop_on_silence);
        assert!(config.daemon.notify_on_error);
        assert_eq!(config.daemon.notify_cooldown_seconds, 45);
        assert!(config.daemon.allowed_processes.is_empty());
        assert_eq!(config.daemon.overlay_command, "cavaii");
        assert!(config.daemon.overlay_args.is_empty());
    }

    #[test]
    fn rejects_visualizer_fade_key_in_main_config() {
        let raw = r#"
        [visualizer]
        fade = true
        "#;

        let err = match parse_config(raw) {
            Ok(_) => panic!("config with visualizer.fade should fail"),
            Err(err) => err,
        };
        let message = err.to_string();
        assert!(message.contains("unknown visualizer key: fade"));
    }

    #[test]
    fn parses_colors_override_gradient() {
        let raw = r#"
        [color]
        gradient = ["rgba(10, 20, 30, 0.8)", "rgba(255, 0, 0, 0.6)", "rgba(0, 255, 0, 0.6)"]
        orientation = "horizontal"
        fade = false
        "#;

        let parsed = match parse_color_overrides(raw) {
            Ok(value) => value,
            Err(err) => panic!("colors override should parse, got error: {err}"),
        };

        let Some(gradient) = parsed.gradient else {
            panic!("missing gradient override");
        };
        assert_eq!(parsed.orientation, Some(ColorOrientation::Horizontal));
        assert_eq!(parsed.fade, Some(false));
        assert_eq!(gradient.len(), 3);
        assert!((gradient[0].r - (10.0 / 255.0)).abs() < 1e-5);
        assert!((gradient[1].r - 1.0).abs() < 1e-5);
        assert!((gradient[2].g - 1.0).abs() < 1e-5);
    }

    #[test]
    fn parses_multiline_gradient_array() {
        let raw = r##"
        name = "theme-name-ignored"
        [color]
        gradient = [
            "#ff0000",
            "rgba(0, 255, 0, 0.7)",
            "#0000ff"
        ]
        "##;

        let parsed = match parse_color_overrides(raw) {
            Ok(value) => value,
            Err(err) => panic!("colors override should parse, got error: {err}"),
        };

        let Some(gradient) = parsed.gradient else {
            panic!("missing gradient override");
        };
        assert_eq!(gradient.len(), 3);
        assert!((gradient[0].r - 1.0).abs() < 1e-5);
        assert!((gradient[1].g - 1.0).abs() < 1e-5);
        assert!((gradient[2].b - 1.0).abs() < 1e-5);
    }

    #[test]
    fn rejects_legacy_color_slots() {
        let raw = r##"
        [color]
        color_1 = "#ff0000"
        "##;

        let err = match parse_color_overrides(raw) {
            Ok(_) => panic!("legacy slot format should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("unknown color key: color_1"));
    }

    #[test]
    fn applies_color_overrides() {
        let mut config = AppConfig::default();
        let raw = r##"
        [color]
        orientation = "horizontal"
        fade = false
        gradient = ["rgba(202, 122, 99, 0.9)", "#ffffff"]
        "##;

        let overrides = match parse_color_overrides(raw) {
            Ok(value) => value,
            Err(err) => panic!("colors override should parse, got error: {err}"),
        };
        apply_color_overrides(&mut config, overrides);

        assert_eq!(config.visualizer.color_orientation, ColorOrientation::Horizontal);
        assert!(!config.visualizer.color_fade);
        assert_eq!(config.visualizer.color_gradient.len(), 2);
        assert!((config.visualizer.color_gradient[0].r - (202.0 / 255.0)).abs() < 1e-5);
        assert!((config.visualizer.color_gradient[0].g - (122.0 / 255.0)).abs() < 1e-5);
        assert!((config.visualizer.color_gradient[0].b - (99.0 / 255.0)).abs() < 1e-5);
        assert!((config.visualizer.color_gradient[0].a - 0.9).abs() < 1e-5);
    }
}
