# Tiled Worldmap Streaming Plan

## Goal

Move the terrain renderer away from loading full 16k height and diffuse maps into memory. The renderer should use generated worldmap tiles for near terrain, compact overview maps for far terrain, and a fixed-size resident GPU cache that updates as the player moves.

The current full-texture shader path does not need to be kept for backwards compatibility. Version control is enough fallback. The existing shader should be updated in place rather than maintaining parallel render paths.

## Starting Assumptions

- Source height map: `16384x16384` R16 little-endian RAW.
- Source diffuse map: `16384x16384` color map.
- Near tile payload size: `1024x1024`.
- Tile padding: `2` pixels copied from neighboring source data.
- Stored tile size: `1028x1028` when padding is included.
- Runtime resident near cache: `5x5` tiles around the player.
- Far height map: `2048x2048` max-height R16.
- Far color overview: `4096x4096` downsampled color map.
- Terrain horizontal and height scale are world metadata stored in the generated manifest.

These values are starting points, not permanent limits.

## Worldmap Package Layout

Packages should be generated under:

```text
assets/worldmaps/<world-name>/
```

Recommended layout:

```text
assets/worldmaps/<world-name>/
  manifest.toml
  height/
    near/
      tile_0000_0000.r16
      tile_0001_0000.r16
      ...
    far/
      max_2048.r16
  color/
    near/
      tile_0000_0000.rgba
      tile_0001_0000.rgba
      ...
    far/
      overview_4096.rgba
```

Raw `.rgba` color tiles are preferred for streaming because they avoid PNG decode spikes.

## Manifest

The pipeline should write a `manifest.toml` with enough metadata for the renderer to validate and load the package without hard-coded asset dimensions.

Example:

```toml
name = "continent"
source_width = 16384
source_height = 16384
horizontal_scale = 0.5
height_scale = 535.5

tile_size = 1024
tile_padding = 2
tile_count_x = 16
tile_count_y = 16

height_format = "r16le"
height_near_path = "height/near"
height_far_path = "height/far/max_2048.r16"
height_far_width = 2048
height_far_height = 2048

color_format = "rgba8"
color_near_path = "color/near"
color_far_path = "color/far/overview_4096.rgba"
color_far_width = 4096
color_far_height = 4096
```

## Pipeline Tool

Add a worldmap build tool as a new Cargo binary:

```text
src/bin/build_worldmap.rs
```

Expected command shape:

```sh
cargo run --release --bin build_worldmap -- \
  --height-input "assets/untracked/continent Height Output 16384.r16" \
  --height-size 16384x16384 \
  --color-input "assets/untracked/continent Material Output 16384_diffuse.png" \
  --output assets/worldmaps/continent \
  --tile-size 1024 \
  --tile-padding 2 \
  --far-height-size 2048x2048 \
  --far-color-size 4096x4096
```

The tool should:

1. Generate padded near R16 height tiles.
2. Generate padded near RGBA8 color tiles.
3. Generate a conservative max-height far R16 map.
4. Generate a far color overview map.
5. Write `manifest.toml`.
6. Validate source dimensions, output dimensions, and exact byte sizes.

This is an offline pipeline that should normally run once per world on a development machine, so it can favor simplicity over streaming its source inputs. The first implementation can load the full source height map and full decoded source color map into memory, then generate outputs one tile or overview map at a time.

For a `16384x16384` world, the raw source memory is roughly:

```text
height R16:   512 MiB
color RGBA8:  1 GiB
```

A realistic pipeline peak of a few GiB is acceptable. A `32768x32768` world would be roughly `2 GiB` for height and `4 GiB` for RGBA color before temporary copies, which is still acceptable for this project if the tool is run on a machine with enough RAM.

The tool should still avoid keeping every generated tile in memory at once. Load the sources, write each generated tile to disk, and keep only the current output tile or overview buffers as working data.

## Tile Padding Rules

Each near tile has:

```text
1024x1024 payload pixels
2px border on all sides
1028x1028 stored pixels
```

For interior tiles, padding comes from neighboring source pixels. For world edges, padding clamps to the nearest valid source pixel.

Padding is required so that:

- Bilinear color samples do not seam at tile edges.
- Height samples near tile boundaries remain continuous.
- Normal samples that read neighboring heights remain stable.
- Ray traversal can safely sample around the current tile.

The manifest should distinguish payload size from stored size. Runtime world coordinates map to payload coordinates; padding is an implementation detail of sampling.

## Runtime Architecture

At startup:

1. Load `assets/worldmaps/<world-name>/manifest.toml`.
2. Load far height map into one R16 texture.
3. Load far color overview into one RGBA8 texture.
4. Allocate fixed-size near height atlas.
5. Allocate fixed-size near color atlas.
6. Load the initial `5x5` tile window centered around the configured start position.
7. Build the CPU collision tile cache for the current tile and immediate neighbors.

For `1024x1024` payload tiles and a `5x5` resident cache:

```text
near height atlas: about 5120x5120 payload pixels, R16
near color atlas:  about 5120x5120 payload pixels, RGBA8
```

If the atlas stores padded tiles directly, atlas dimensions become:

```text
5 * 1028 = 5140
```

Either layout can work. Storing padded tiles directly is simpler. Packing only payload pixels and handling padding in shader is more complex and probably not worth it initially.

## Resident Tile Cache

Use a fixed `5x5` rolling cache around the player. The cache should keep metadata for each atlas slot:

```text
slot_x
slot_y
world_tile_x
world_tile_y
loaded
loading
last_used_frame
```

Use ring placement:

```text
slot_x = world_tile_x mod 5
slot_y = world_tile_y mod 5
```

When the player crosses into a new tile, only newly visible rows or columns need to be loaded and uploaded. Existing tiles remain in place. The renderer should never shift or copy the whole atlas.

The shader must be able to tell whether the tile in a slot is actually the requested world tile. A modulo slot can contain stale data from another world coordinate.

Pass this as a small lookup resource, such as:

- a tiny integer texture with resident world tile coordinates, or
- a uniform array if SDL GPU and shader limits make that straightforward.

For a `5x5` cache, either is fine.

## Shader Changes

Update `shaders/voxelspace.frag` in place.

The main raymarching structure should remain recognizable:

- ray setup
- terrain bounds interval
- near DDA
- distance-scaled raymarch
- hit refinement
- backdrop raycast
- lighting, fog, and debug colors

The major change is the data access layer.

Current model:

```text
height_near_map: one global dense height texture
height_far_map: one global far height texture
color_map: one global color texture
```

Target model:

```text
height_near_atlas: fixed resident tile atlas
color_near_atlas: fixed resident tile atlas
height_far_map: global far max-height texture
color_far_map: global far overview texture
tile_residency: small lookup for atlas slots
```

Add helper functions around tile resolution:

```glsl
world_tile_for_position(...)
atlas_slot_for_tile(...)
tile_is_resident(...)
near_tile_uv(...)
sample_near_height(...)
sample_far_height(...)
sample_height(...)
sample_near_color(...)
sample_far_color(...)
sample_color(...)
```

Most shader code should continue calling high-level helpers such as:

```glsl
height_at(world_pos, lod_blend)
color_at(world_pos)
```

`raycast_near_height_cells()` needs the largest change because it currently assumes one global near height texture. It should traverse world cells, resolve those cells to resident tile slots, and stop using near DDA once the ray leaves resident near data. The far height map raymarch/backdrop should continue from there.

## Sampling And Fallback Behavior

Height sampling:

```text
if near tile is resident and close enough:
  sample near atlas
else:
  sample far max-height map
```

During LOD blending:

```text
near resident: blend near detailed height to far max-height
near missing: use far max-height only
```

Color sampling:

```text
if near color tile is resident and close enough:
  sample near color atlas
else:
  sample far color overview
```

To avoid visible tile load pops, consider a per-tile fade-in once the basic implementation works. The first version can rely on distance blending and the far overview fallback.

## CPU Collision

The current collision height field cannot remain a full 16k CPU allocation.

Replace it with a CPU-side height tile cache:

- Keep the player tile and neighboring tiles resident.
- Synchronously load the tile needed for the current player position before collision uses it.
- Use the detailed near R16 tile data for ground height.
- Fall back conservatively only if the player is outside valid tile bounds.

The collision cache does not need to match the full GPU `5x5` cache, but sharing loaded tile data before upload would avoid duplicate disk reads.

## Streaming And Uploads

Use worker threads for disk reads and CPU decode. GPU texture uploads should happen on the render thread.

Frame pacing rules:

- Cap how many tile uploads can happen per frame.
- Prioritize tiles closest to the player.
- Load the tile containing the player first.
- Then load the cross or ring of nearby tiles.
- Avoid blocking the render loop except for collision-critical current-tile data.

The first version can be simpler and synchronous, but the design should keep the upload boundary clear so async loading can be added without rewriting the renderer.

## Config Impact

Add config settings for:

```toml
worldmap = "assets/worldmaps/continent/manifest.toml"
tile_cache_radius = 2
```

World scale should come from the worldmap manifest, not `config.toml` and not hard-coded renderer constants. This keeps a generated world package self-describing and lets different worlds use different horizontal or vertical scale without recompiling.

Existing render tuning settings should continue to apply:

- `ray_iteration_count`
- `performance_render_scale`
- `near_dda_distance`
- `near_dda_max_steps`
- `height_lod_blend_start`
- `height_lod_blend_end`
- `normal_detail_blend_start`
- `normal_detail_blend_end`

## Documentation Tasks

Add a dedicated worldmap format document:

```text
docs/worldmaps.md
```

It should cover:

- Directory layout.
- Manifest fields.
- Height tile format.
- Color tile format.
- Padding rules.
- Far max-height map behavior.
- Far color overview behavior.
- How to run the build pipeline.
- How to point `config.toml` at a generated worldmap.
- Expected disk and VRAM sizes for the default settings.

Update the existing terrain renderer document:

```text
docs/terrain-renderer.md
```

It should describe:

- Tiled near terrain data.
- Fixed resident atlas.
- Far map fallback.
- Tile residency lookup.
- How LOD blending works with missing near tiles.
- How collision uses tiled height data.
- Updated debug visual meanings if the debug modes change.

Update `README.md` with the short command examples only. Keep detailed explanation in `docs/`.

## Implementation Order

1. Define `manifest.toml` schema and Rust structs.
2. Implement the worldmap build pipeline for height tiles and far max-height.
3. Add color tile and far color overview generation.
4. Add documentation for the worldmap package format and pipeline usage.
5. Change runtime startup to load the worldmap manifest and far maps.
6. Allocate near height/color atlases.
7. Implement initial synchronous `5x5` tile loading around the start position.
8. Update shader bindings and helper functions for tiled sampling.
9. Update `raycast_near_height_cells()` for resident tile lookup.
10. Replace full CPU collision height field with a tiled collision cache.
11. Add movement-triggered tile streaming and per-frame upload caps.
12. Update `docs/terrain-renderer.md`.
13. Tune tile size, cache radius, and upload budget based on measurements.

## Risks

- Shader tile lookup can add overhead. Keep lookup data tiny and branch behavior simple.
- Tile-edge seams are likely if padding or coordinate mapping is even slightly wrong.
- GPU upload spikes can hurt frame pacing if too many tiles upload in one frame.
- PNG decode may be too slow for runtime color tiles.
- Near DDA can become incorrect if it crosses into stale or missing atlas slots.
- Collision must not sample missing near data under the player.

## Initial Success Criteria

- App starts without loading full 16k maps.
- Far terrain renders from the 2048 max-height map and far color overview.
- Near terrain around the player renders from tiled height and color atlases.
- Moving across tile boundaries loads new tiles without full-atlas reshuffles.
- Collision works from detailed tiled height data.
- No obvious seams at tile boundaries.
- Documentation explains how to build and use a worldmap package.
- The replay-camera profiling path runs successfully:
```sh
cargo run --bin tungsten -- --replay-camera recordings/camera-1782756981829.tsv
```
- FPS statistics from the tiled implementation are not *drastically* worse than the current full-texture renderer on the same machine and config.
- Any expected performance tradeoffs are documented with the benchmark output used for comparison.

Replay output pre-changes:

```
replay complete
frames: 11921
warmup_frames_ignored: 10
elapsed_seconds: 69.261487
average_fps: 172.116
min_fps: 112.596
max_fps: 199.960
frame_ms_min: 5.001
frame_ms_avg: 5.810
frame_ms_max: 8.881
fps_csv: /tmp/tungsten-replay-fps-1782758471650.csv
```
