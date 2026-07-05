# tungsten

## Runtime Config

Runtime rendering knobs live in `config.toml`:

```toml
worldmap = "assets/worldmaps/continent/manifest.toml"
tile_cache_radius = 1

ray_iteration_count = 700
performance_render_scale = 0.5
present_mode = "vsync" # "vsync", "immediate", or "mailbox"
max_framerate = 0.0 # 0.0 means unlimited
render_debug_visuals = false
near_dda_distance = 512.0
near_dda_max_steps = 1024

raster_cube_enabled = false
raster_cube_x = 320.0
raster_cube_y = 240.0
raster_cube_height = 120.0
raster_cube_size = 64.0

start_x = 250.0
start_y = 330.0
start_height = 150.0

height_lod_blend_start = 125.0
height_lod_blend_end = 300.0

normal_detail_blend_start = 500.0
normal_detail_blend_end = 1000.0
```

The config is loaded once at startup. Missing keys fall back to the built-in defaults. `worldmap` points at a generated package manifest; see [docs/worldmaps.md](docs/worldmaps.md). Use `present_mode = "immediate"` for raw throughput measurement, `present_mode = "mailbox"` for low-latency no-tear presentation where supported, or `present_mode = "vsync"` for display-paced presentation. Set `max_framerate` above `0.0` to add a CPU-side frame cap. Set `render_debug_visuals = true` to enable cycling terrain debug views with `F3`. Set `raster_cube_enabled = true` to draw the test cube before the upscale pass; its `x/y/height/size` values are in world units.

## Controls

Press `G` to toggle between freecam and gravity/player movement.

In gravity mode, `WASD` moves along the terrain and `Space` jumps. Scroll adjusts camera height, and `Shift` + scroll adjusts movement speed. Freecam keeps the original controls.

When `render_debug_visuals` is enabled, `F3` cycles through no debug view, height source colors, ray/hit method colors, normal-lighting mode colors, and scene depth output.

Press `F11` to start recording a camera trace, and press `F11` again to stop. Recordings are written under `recordings/` as TSV files with `frame x y height yaw pitch` samples every 10 submitted frames.

Replay a recorded trace as a fullscreen benchmark:

```sh
cargo run --release -- --replay-camera recordings/camera-0000000000000.tsv
```

Replay honors `present_mode` and `max_framerate` from `config.toml`, interpolates between recorded samples, exits after the last sample, and writes FPS statistics to stdout. The stdout summary ignores the first replay frames as warmup. It also writes a graph-friendly CSV under `/tmp/tungsten-replay-fps-*.csv` with average/min/max FPS buckets for each 10 replay frames.

## Tools

Generate a tiled worldmap package:

```sh
cargo run --release --bin build_worldmap -- \
  --height-input "assets/untracked/continent Height Output 16384.r16" \
  --height-size 16384x16384 \
  --color-input "assets/untracked/continent Material Output 16384_diffuse.png" \
  --output assets/worldmaps/continent \
  --tile-size 1024 \
  --tile-padding 2 \
  --far-height-size 2048x2048 \
  --far-color-size 4096x4096 \
  --horizontal-scale 0.5 \
  --height-scale 535.5 \
  --name continent
```

Generate a conservative max-height R16 mip:

```sh
cargo run --release --bin max_height_mip -- \
  --input "assets/untracked/continent Height Output 8192.r16" \
  --input-size 8192x8192 \
  --output "assets/untracked/continent Height Max 1024.r16" \
  --output-size 1024x1024
```

Generate a bilinear upsampled R16 heightmap:

```sh
cargo run --release --bin upsample_heightmap -- \
  --input "assets/untracked/continent Height Output 8192.r16" \
  --input-size 8192x8192 \
  --output "assets/untracked/continent Height Output 16384 interpolated.r16" \
  --output-size 16384x16384
```

Generate a dithered bilinear upsampled colormap:

```sh
cargo run --release --bin upsample_colormap -- \
  --input "assets/untracked/continent Material Output 4096_diffuse.png" \
  --output "assets/untracked/continent Material Output 16384_diffuse_dithered.png" \
  --output-size 16384x16384
```
