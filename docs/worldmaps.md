# Worldmaps

Worldmaps are generated terrain packages used by the renderer. They let the app keep a small resident near-terrain cache in VRAM while still rendering a larger source world.

## Layout

Worldmap packages live under:

```text
assets/worldmaps/<world-name>/
```

Default generated layout:

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

## Manifest

`manifest.toml` is a flat key/value file. Example:

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

`horizontal_scale` converts source pixels to world-space X/Z units. `height_scale` converts normalized R16 samples to world-space height. These are world properties, not renderer config values.

## Tile Format

Near height tiles are raw little-endian R16 files. Near color tiles are raw RGBA8 files.

With the default settings, each tile has:

```text
payload: 1024x1024 source pixels
padding: 2 pixels on every side
stored:  1028x1028 pixels
```

Padding is copied from neighboring source pixels. At world edges it is clamped to the nearest valid source pixel. The renderer samples inside this padded tile data to avoid visible seams and to keep normal/height samples stable near tile boundaries.

## Far Maps

The far height map is a conservative max-height R16 map. Each far texel stores the maximum source height in its covered source region, so distant raymarching is less likely to miss peaks.

The far color overview is a downsampled raw RGBA8 map. It is always resident and is used when a near color tile is not resident.

## Build Pipeline

Generate a worldmap package with:

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

The tool loads the full source height and decoded source color map into memory, then writes generated outputs one tile or overview map at a time.

## Runtime Config

Point the renderer at a generated worldmap:

```toml
worldmap = "assets/worldmaps/continent/manifest.toml"
tile_cache_radius = 1
```

`tile_cache_radius = 1` creates a `3x3` resident near-tile cache. The runtime uploads only the `1024x1024` tile payloads into the near atlases; the generated `2px` padding is used when extracting tile payloads and for CPU collision data. Tiles live in ring atlas slots, so moving across a tile boundary uploads only newly visible slots while shared tiles stay resident. `tile_cache_radius = 2` is supported for a `5x5` cache with higher VRAM use and larger tile-update bursts.

Approximate default VRAM:

```text
near height atlas, R16:   18 MiB
near color atlas, RGBA8:  36 MiB
far height, R16 2048:     8 MiB
far color, RGBA8 4096:   64 MiB
```
