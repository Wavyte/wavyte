# wavyte (v0.2.1)

Programmatic video composition and rendering in Rust.

Wavyte is a library-first engine that turns a timeline composition (JSON or Rust DSL) into pixels
through a deterministic pipeline:

1. Evaluate timeline state for a frame.
2. Compile to backend-agnostic `RenderPlan` IR.
3. Execute passes on the CPU backend.
4. Optionally encode MP4 through system `ffmpeg`.

Repository layout is now a Cargo workspace:
- `wavyte-core`: engine/library crate (exports crate name `wavyte`).
- `wavyte-cli`: CLI crate (binary `wavyte`).
- `bench`: standalone benchmark crate.

## What you get in v0.2.1

- Immutable prepared asset store (`PreparedAssetStore`) with deterministic asset IDs.
- CPU rendering backend (`vello_cpu`) with premultiplied RGBA semantics.
- Track layout primitives: `Absolute`, `HStack`, `VStack`, `Grid`, `Center`.
- Transition/effect pipeline (`crossfade`, `wipe`, `blur`, inline opacity/transform effects).
- Chunked parallel rendering with optional static-frame elision.
- Optional media decode/probe + audio mix/mux via `media-ffmpeg` feature.
- Hot-loop optimizations in pipeline/backend:
  - one-time composition validation at render API boundaries,
  - cached effect/transition parsing during compile with hashed cache keys,
  - fingerprints computed only when static-frame elision is enabled,
  - reusable CPU surfaces across frames/chunks.
- Improved media behavior:
  - batched video decode prefetch in CPU backend,
  - linear-interpolated audio resampling in mixer,
  - robust ffmpeg stderr draining during encode.

## Install and prerequisites

Rust:

```bash
rustc --version
```

MP4 rendering requires `ffmpeg` on `PATH`:

```bash
ffmpeg -version
ffprobe -version
```

## Quick start

Run examples:

```bash
cargo run -p wavyte-core --example render_crossfade_png
cargo run -p wavyte-core --example render_blur_png
cargo run -p wavyte-core --example render_remotion_hello_world_mp4
cargo run -p wavyte-core --example render_aesthetic_motion_mp4
cargo run -p wavyte-core --example render_aesthetic_fx_mp4
cargo run -p wavyte-core --example render_aesthetic_layout_mp4
```

Full media/layout example (`media-ffmpeg` feature):

```bash
cargo run -p wavyte-core --features media-ffmpeg --example render_full_gamut_media_layout_mp4
```

Examples write outputs into repo-local `assets/`.

## CLI

Render one PNG frame from JSON:

```bash
cargo run -p wavyte-cli --bin wavyte -- frame --in comp.json --frame 0 --out out.png
```

Render MP4 from JSON:

```bash
cargo run -p wavyte-cli --bin wavyte -- render --in comp.json --out out.mp4
```

Diagnostics:

- `--dump-fonts`: resolved text family + font SHA-256.
- `--dump-svg-fonts`: SVG text node count + loaded SVG font face count.

## Minimal JSON composition

Save as `comp.json`:

```json
{
  "fps": { "num": 30, "den": 1 },
  "canvas": { "width": 512, "height": 512 },
  "duration": 60,
  "assets": {
    "rect": {
      "Path": { "svg_path_d": "M0,0 L120,0 L120,120 L0,120 Z" }
    }
  },
  "tracks": [
    {
      "name": "main",
      "z_base": 0,
      "clips": [
        {
          "id": "c0",
          "asset": "rect",
          "range": { "start": 0, "end": 60 },
          "props": {
            "transform": {
              "Keyframes": {
                "keys": [
                  {
                    "frame": 0,
                    "value": {
                      "translate": { "x": 180.0, "y": 180.0 },
                      "rotation_rad": 0.0,
                      "scale": { "x": 2.5, "y": 2.5 },
                      "anchor": { "x": 0.0, "y": 0.0 }
                    },
                    "ease": "Linear"
                  }
                ],
                "mode": "Hold",
                "default": null
              }
            },
            "opacity": {
              "Keyframes": {
                "keys": [{ "frame": 0, "value": 1.0, "ease": "Linear" }],
                "mode": "Hold",
                "default": null
              }
            },
            "blend": "Normal"
          },
          "z_offset": 0,
          "effects": [],
          "transition_in": null,
          "transition_out": null
        }
      ]
    }
  ],
  "seed": 1
}
```

Render:

```bash
cargo run -p wavyte-cli --bin wavyte -- frame --in comp.json --frame 0 --out out.png
```

## Library usage

Core units:

- `Composition`: timeline model.
- `PreparedAssetStore`: immutable prepared assets.
- `Evaluator`: per-frame visibility + resolved clip state.
- `RenderPlan`: backend-agnostic pass graph.
- `RenderBackend`: pass executor.

Main APIs (`wavyte-core/src/render/pipeline.rs`):

- `render_frame(...) -> FrameRGBA`
- `render_frames_with_stats(...) -> (Vec<FrameRGBA>, RenderStats)`
- `render_to_mp4_with_stats(...) -> RenderStats`

Validation behavior:
- public render APIs validate composition once per call,
- per-frame hot loops use internal unchecked evaluation after initial validation.

Backend creation:

```rust
let settings = wavyte::RenderSettings {
    clear_rgba: Some([18, 20, 28, 255]),
};
let mut backend = wavyte::create_backend(wavyte::BackendKind::Cpu, &settings)?;
```

## Rendering semantics

Premultiplied alpha is the core pixel contract:

- decoded images are premultiplied at ingest,
- render outputs are premultiplied `FrameRGBA`,
- compositing and blur assume premultiplied data.

`RenderPlan` is the stable evaluate/compile/render boundary:

- `Pass::Scene`: draw operations into a surface,
- `Pass::Offscreen`: post-effect pass (for example blur),
- `Pass::Composite`: combine layer surfaces (`Over`, `Crossfade`, `Wipe`).

## Assets and determinism

Asset path rules:

- must be relative,
- path separators normalized,
- no `..` traversal.

Preparation behavior:

- `PreparedAssetStore::prepare` front-loads IO/decoding.
- `AssetId` is deterministic from normalized asset key + params.
- Renderers are IO-free and consume only prepared assets.

## Parallel rendering

`RenderThreading` controls execution:

- `parallel`: enable frame-parallel execution,
- `chunk_size`: chunk granularity,
- `threads`: optional fixed worker count,
- `static_frame_elision`: fingerprint-based dedupe within chunk.

Parallel mode uses worker-local CPU backends and preserves output frame order.
In MP4 mode, static-frame elision reuses unique rendered frames during encode without cloning
duplicate frame buffers.

CPU video decode cache knobs (optional env vars):
- `WAVYTE_VIDEO_CACHE_CAPACITY` (default `64`)
- `WAVYTE_VIDEO_PREFETCH_FRAMES` (default `12`)

## Media and audio

Enable with:

```bash
cargo run -p wavyte-core --features media-ffmpeg --example render_full_gamut_media_layout_mp4
```

Capabilities:

- video/audio trim, playback rate, volume, fades, mute,
- video frame decode for rendering,
- audio mix to `f32le` and mux during MP4 encode.

## MP4 encoding

Wavyte wraps system `ffmpeg` (`wavyte-core/src/encode/ffmpeg.rs`):

- raw RGBA frames streamed to stdin,
- optional mixed audio input,
- mp4 output (`libx264`, `yuv420p`, optional `aac`).
- stderr is drained concurrently and surfaced on non-zero exit.
- current MP4 API requires integer FPS (`fps.den == 1`) and even frame dimensions.

If `ffmpeg` is unavailable, encoding fails explicitly.

## Project layout

- `wavyte-core/src/foundation/`: errors, core types, shared math/hash helpers.
- `wavyte-core/src/animation/`: anim/ease/procedural/ops.
- `wavyte-core/src/transform/`: linear, affine, non-linear helper modules.
- `wavyte-core/src/effects/`: effect parse/normalize + blur/composite/transitions.
- `wavyte-core/src/layout/`: layout solver.
- `wavyte-core/src/composition/`: model + DSL builders.
- `wavyte-core/src/eval/`: evaluator/frame graph.
- `wavyte-core/src/compile/`: render IR/plan + fingerprinting.
- `wavyte-core/src/render/`: backends, pass execution, CPU impl, pipeline.
- `wavyte-core/src/audio/`: manifest/mixer.
- `wavyte-core/src/encode/`: ffmpeg encoder wrapper.
- `wavyte-core/src/assets/`: asset prepare/decode/media/raster helpers.
- `wavyte-cli/src/main.rs`: CLI entrypoint.
- `bench/src/main.rs`: standalone benchmark harness.
- `EXPLANATION.md`: exhaustive architecture deep dive.

## Release readiness gate

MSRV: Rust `1.93` (edition `2024`).

Recommended gate:

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features --release
cargo test -p wavyte-bench --release
```

## License

AGPL-3.0-only (`LICENSE`).
