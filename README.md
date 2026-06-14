# tungsten

## Runtime Config

Runtime rendering knobs live in `config.toml`:

```toml
ray_iteration_count = 700
performance_render_scale = 0.5

start_x = 250.0
start_y = 330.0
start_height = 150.0

height_lod_blend_start = 125.0
height_lod_blend_end = 300.0

normal_detail_blend_start = 500.0
normal_detail_blend_end = 1000.0
```

The config is loaded once at startup. Missing keys fall back to the built-in defaults.

## Controls

Press `G` to toggle between freecam and gravity/player movement.

In gravity mode, `WASD` moves along the terrain and `Space` jumps. Scroll adjusts camera height, and `Shift` + scroll adjusts movement speed. Freecam keeps the original controls.

## Tools

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
