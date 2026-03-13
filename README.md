# Cavaii

Cavaii is a lightweight GTK overlay audio visualizer with an optional daemon that starts/stops the
overlay based on audio activity. It renders bars or a smooth wave, with optional GPU rendering.

## Build

```
cargo build --release
```

This produces `cavaii` (overlay) and `cavaii-daemon`.

## Run

```
./target/release/cavaii
./target/release/cavaii-daemon
```

You can point to a custom config with `--config <path>` or `--config=/path/to/config.toml`.

Default config path is `$XDG_CONFIG_HOME/cavaii/config.toml` or `~/.config/cavaii/config.toml`.
`colors.toml` lives next to the main config and overrides colors.

## Configuration

### `config.toml` defaults

`[overlay]`
- `layer = "background"`
- `anchor_margin = 10`
- `width = 1000`
- `height = 300`

`[visualizer]`
- `backend = "pipewire"` (auto | pipewire | cava | dummy)
- `type = "bar"` (bar | wave)
- `bars = 120`
- `bar_width = 12`
- `bar_corner_radius = 20`
- `wave_thickness = 2`
- `gap = 5`
- `framerate = 60`
- `gpu = "enabled"`
- `pipewire_attack = 0.14`
- `pipewire_decay = 0.975`
- `pipewire_gain = 1.20`
- `pipewire_curve = 0.95`
- `pipewire_neighbor_mix = 0.24`

`[daemon]`
- `enabled = true`
- `poll_interval_ms = 90`
- `activity_threshold = 0.035`
- `activate_delay_ms = 180`
- `deactivate_delay_ms = 2200`
- `stop_on_silence = true`
- `notify_on_error = true`
- `notify_cooldown_seconds = 45`
- `allowed_processes = []`
- `overlay_command = "cavaii"`
- `overlay_args = []`

### `colors.toml`

`colors.toml` is the only place for color settings:

```
[color]
orientation = "vertical"
fade = true
gradient = ["rgba(175, 198, 255, 0.7)"]
```

Supported keys in `colors.toml`:
- `[color] orientation = "vertical" | "horizontal"` (default: `vertical`)
- `[color] fade = true | false` (default: `true`)
- `[color] gradient = [ ... ]` with 1 to 5 colors

If `gradient` has one color, Cavaii renders a solid color.

If no color override is provided, built-in defaults are used.

## Notes

- GPU rendering uses GTK 4.14+ APIs.
- Wave rendering automatically downsamples to reduce per-frame work.
- `assets/themes/*.toml` contains presets in the `[color]` format (`orientation`, `fade`, `gradient`).
