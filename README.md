# Cavaii

Cavaii is a lightweight overlay audio visualizer for Wayland based on CAVA and GTK with an optional daemon that starts/stops the overlay based on audio activity.
It renders bars or a smooth wave, with optional GPU rendering.
It's a heavily modified version of [Kwybars](https://github.com/naurissteins/Kwybars) to meet my personal usage preferences.

## Build

```
cargo build --release
```

This produces `cavaii` (overlay) and `cavaii-daemon`.

## Run

To run it globally, place
- `./target/release/cavaii`
- `./target/release/cavaii-daemon`
in a folder visible for your shell path, like `~/.local/bin/`.

Configuration path is `~/.config/cavaii/` and the config files are:
- `~/.config/cavaii/config.toml`
- `~/.config/cavaii/colors.toml`
If either file is missing, Cavaii creates it on first use with built-in defaults.

Both `cavaii` and `cavaii-daemon` logs are written to `~/.local/state/cavaii.log`.

## Project structure

```
crates/
  common/   # shared config, logging, notifications, spectrum frame types
  engine/   # live frame sources (cava + dummy interface for testing)
    src/live/
      mod.rs
      cava.rs
      dummy.rs
  overlay/  # GTK renderer and UI wiring
  daemon/   # activity watcher and overlay process lifecycle
assets/
  config.toml
  colors.toml
  themes/
```

Under [themes](./assets/themes/) there are bundled color presets.

## Configuration

### `config.toml` defaults

- `logging = false`

`[overlay]`
- `anchor_margin = 10`
- `width = 800`
- `height = 100`

`[visualizer]`
- `backend = "cava"` (cava | dummy)
- `type = "bars"` (bars | wave)
- `framerate = 30`
- `gpu = "enabled"`

`[bar]`
- `points = 140`
- `point_width = 12`
- `point_gap = 4`
- `corner_radius = 20`

`[wave]`
- `points = 30`
- `point_width = 12`
- `point_gap = 20`
- `thickness = 4`

`[daemon]`
- `poll_interval_ms = 500`
- `activity_threshold = 0.035`
- `activate_delay_ms = 0`
- `deactivate_delay_ms = 10`
- `stop_on_silence = true`
- `notify_on_error = true`
- `notify_cooldown_seconds = 120`
- `allowed_processes = ["spotify", "firefox"]`


### `colors.toml`

`colors.toml` is the only place for color settings:

```
[color]
orientation = "horizontal"
fade = true
gradient = ["rgba(175, 198, 255, 0.7)"]
```

Supported keys in `colors.toml`:
- `[color] orientation = "horizontal" | "vertical" | "height"` (default: `horizontal`)
- `[color] fade = true | false` (default: `true`)
- `[color] gradient = [ ... ]` with 1 to 5 colors

If no color override is provided, built-in defaults are used.

## Performance summary

The benchmark command is:

```
cargo test --release
```

This runs correctness checks and the benchmark harness.

Optional knobs:

- `CAVAII_BENCH_PARSE_ITERS` (default `3000000`)
- `CAVAII_BENCH_DUMMY_ITERS` (default `900000`)
- `CAVAII_BENCH_BAR_COUNT` (default `120`)

The benchmark prints:

- workload timing + throughput
- CPU usage min/avg/max
- GPU usage + GPU memory min/avg/max (when `nvidia-smi` is available)
- RSS memory min/avg/max
- process I/O deltas (`read_bytes`, `write_bytes`)
- context-switch deltas

The benchmark results for my machine are:

- `perf_parse_cava_raw_u16_frame`: 1,000,000 iterations (~`0.031us/iter`, about `31ms` total)
- `perf_dummy_source_generation`: 300,000 iterations (~`1.174us/iter`, about `352ms` total)
- wall time: `0.485s`
- CPU user/system: `0.454s / 0.058s` (approx `105.67%` CPU)
- RAM peak (`max_rss`): `84,012 KB`
- block I/O: read `0`, write `0`
- context switches: voluntary `67`, involuntary `23`
- GPU utilization: avg `12%`, max `12%`
- GPU memory: avg/max `553 MiB`

Remember that these are comparative metrics generated from my environment, not universal values.
