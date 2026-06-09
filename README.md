# tungsten

## Runtime Profile

The app defaults to the performance profile, which renders terrain at half window resolution and nearest-upscales it.

Use the native-resolution profile with:

```sh
TUNGSTEN_PROFILE=quality cargo run
```

Tune the performance profile in `src/main.rs` with `PERFORMANCE_RENDER_SCALE`.

## Tools

Generate a conservative max-height R16 mip:

```sh
cargo run --release --bin max_height_mip -- \
  --input "assets/untracked/continent Height Output 8192.r16" \
  --input-size 8192x8192 \
  --output "assets/untracked/continent Height Max 1024.r16" \
  --output-size 1024x1024
```
