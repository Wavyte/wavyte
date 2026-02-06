# wavyte (v0.2.0)

Programmatic video composition and rendering in Rust.

Wavyte is a library-first engine that turns a **timeline composition** (JSON or a Rust builder DSL)
into pixels via a deterministic pipeline:

1. **Evaluate** the timeline at a frame index → visible clips in painter’s order
2. **Compile** into a backend-agnostic **RenderPlan**
3. **Render** the plan on the **CPU backend**
4. Optionally **encode MP4** by streaming frames to the system `ffmpeg` binary

Wavyte v0.2.0 adds immutable prepared assets, chunked parallel rendering, static-frame elision,
feature-gated media decode/mux (`media-ffmpeg`), and track layout primitives.

---

## Table of contents

- Getting started
  - Prerequisites
  - Run an example
  - Minimal composition JSON (copy/paste)
- CLI
- Library usage (pipeline and API map)
- Backends and features
- Assets (prepared store, determinism)
- Rendering semantics (`RenderPlan`, premultiplied alpha)
- Parallel/chunked rendering
- Video/audio + layout
- MP4 encoding (`ffmpeg`)
- Development and quality gate
- License

---

## Getting started

### Prerequisites

PNG output works out of the box.

MP4 output requires a working `ffmpeg` on `PATH`:

```bash
ffmpeg -version
```

### Run an example (repo)

CPU is the default (no feature flags required):

```bash
cargo run --example render_crossfade_png
cargo run --example render_blur_png
cargo run --example render_aesthetic_motion_mp4
cargo run --example render_aesthetic_fx_mp4
cargo run --example render_aesthetic_layout_mp4
```

MP4 example (requires `ffmpeg`):

```bash
cargo run --example render_to_mp4
```

Full media/layout gamut example (requires `ffmpeg` + `ffprobe` and `media-ffmpeg`):

```bash
cargo run --features media-ffmpeg --example render_full_gamut_media_layout_mp4
```

In this repo, examples write outputs into the repo-local `assets/` directory (intentionally untracked).

### Minimal composition JSON (copy/paste)

This JSON uses only an inlined `Path` asset (no external files), so you can render it immediately.
Save it as `comp.json`:

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

Render it via the CLI:

```bash
cargo run --bin wavyte -- frame --in comp.json --frame 0 --out out.png
```

---

## CLI

The repository builds a `wavyte` binary (always built; no feature flag).

Render a single frame PNG from a JSON composition:

```bash
cargo run --bin wavyte -- frame --in comp.json --frame 0 --out out.png
```

Render an MP4 from a JSON composition (requires `ffmpeg`):

```bash
cargo run --bin wavyte -- render --in comp.json --out out.mp4
```

Backend selection:

- CPU only in v0.2 (`--backend cpu`)

Implementation details (useful for debugging):

- The CLI validates the composition before rendering.
- The CLI prepares a `PreparedAssetStore` rooted at the directory containing the `--in` JSON file.
- Debug font resolution:
  - `--dump-fonts` prints resolved text font family + SHA-256 of the font bytes.
  - `--dump-svg-fonts` prints SVG text node count + SVG fontdb face count (system + project fonts).

---

## Library usage

Wavyte’s core units are:

- `Composition`: the timeline (assets + tracks + clips)
- `Evaluator`: resolves per-frame visibility and clip properties
- `RenderPlan`: backend-agnostic render IR
- `RenderBackend`: executes the IR into pixels
- `PreparedAssetStore`: immutable prepared assets (IO/decoding front-loaded)

The main convenience functions are in `src/render/pipeline.rs`:

- `render_frame(&Composition, FrameIndex, backend, &PreparedAssetStore) -> FrameRGBA`
- `render_frames_with_stats(..., &RenderThreading) -> (Vec<FrameRGBA>, RenderStats)`
- `render_to_mp4_with_stats(..., RenderToMp4Opts { threading, .. }) -> RenderStats`

Backend creation:

```rust
let settings = wavyte::RenderSettings {
    clear_rgba: Some([18, 20, 28, 255]),
};
let mut backend = wavyte::create_backend(wavyte::BackendKind::Cpu, &settings)?;
```

Asset IO is isolated:

- Renderers never read from disk directly.
- All external assets are prepared up front with `PreparedAssetStore::prepare`.

For a guided tour of the public API, see the module map in “Project layout” below and the crate
documentation on docs.rs.

---
## Migration Notes (v0.1 -> v0.2)

- `AssetCache` and `FsAssetCache` were removed.
  Use `PreparedAssetStore::prepare(&comp, root)` and pass `&PreparedAssetStore` through pipeline APIs.
- New parallel/chunked APIs:
  `render_frames_with_stats` and `render_to_mp4_with_stats` with `RenderThreading`.
- `VideoAsset` and `AudioAsset` now support trim/rate/volume/fades/mute controls.
- `Track` now supports layout fields:
  `layout_mode`, `layout_gap_px`, `layout_padding`, `layout_align_x`, `layout_align_y`, `layout_grid_columns`.

---

## Backends

### CPU backend (default)

Always available.

- Raster engine: `vello_cpu`
- SVG support: `usvg` parse + `resvg` rasterize (premultiplied RGBA8)

---

## Assets (prepared store, determinism)

External asset sources (image/svg/text font) must be:

- relative paths (no leading `/`)
- OS-agnostic (backslashes normalized to `/`)
- free of `..`

Wavyte enforces these rules during validation and asset key normalization.

Preparation:

- `AssetId` is stable and derived from normalized asset key + parameters.
- `PreparedAssetStore::prepare` eagerly loads image/svg/text/path and media assets.
- Validation checks path shape; IO/decode failures surface during prepare.

## Parallel/chunked rendering

- `RenderThreading` controls parallelism (`parallel`, `chunk_size`, `threads`).
- Parallel mode uses worker-local CPU backends via rayon.
- Optional static-frame elision deduplicates identical frame fingerprints in a chunk.

## Video/audio + layout

- Enable media decode/probe with `--features media-ffmpeg`.
- Video/audio assets support trim/rate/volume/fades/mute.
- Audio is mixed to `f32le` and muxed into MP4 when present.
- Track layouts support `Absolute`, `HStack`, `VStack`, `Grid`, `Center`.

---

## Rendering semantics

### Premultiplied alpha (non-negotiable)

Wavyte’s internal and output pixel convention is **premultiplied RGBA8**:

- decoded images are premultiplied at ingest
- render backends output premultiplied RGBA8 `FrameRGBA`
- compositing operations assume premultiplied alpha

If you integrate Wavyte with other systems, this is the single most important detail to preserve
to avoid halos and incorrect blends.

### The `RenderPlan` (the backend boundary)

`RenderPlan` is the stable boundary between “timeline evaluation” and “pixel backends”.

It is a sequence of passes over explicit render surfaces:

- `Pass::Scene(ScenePass)` draws ops into a surface
- `Pass::Offscreen(OffscreenPass)` renders an effect into a new surface (e.g. blur)
- `Pass::Composite(CompositePass)` combines surfaces into a destination (over/crossfade/wipe)

This separation keeps compile and render concerns decoupled and testable.

---

## MP4 encoding and `ffmpeg`

Wavyte encodes MP4 by invoking the system `ffmpeg` binary and streaming raw RGBA frames to stdin.
This is intentionally a **runtime prerequisite**, not a Cargo feature.

Where this lives:

- `src/render/encode_ffmpeg.rs`: `FfmpegEncoder` + config validation + process spawning
- `src/render/pipeline.rs`: orchestration via `render_to_mp4`

Behavior:

- If `ffmpeg` is not found on `PATH`, encoding returns an error (no silent fallback).
- Frames are rendered as premultiplied RGBA8. The encoder can “flatten” alpha over a background
  color (see `EncodeConfig` / `default_mp4_config`).

---

## Project layout (where to look)

- `src/composition/model.rs`: `Composition`, assets, tracks, clips, validation rules
- `src/animation/*.rs`: `Anim<T>` and helpers (keyframes, sequencing, easing)
- `src/composition/eval.rs`: `Evaluator` and the evaluated graph (visibility + resolved transforms)
- `src/render/compile.rs`: backend-agnostic compiler into `RenderPlan`
- `src/render/backend.rs`: backend trait + backend selection (`BackendKind`, `create_backend`)
- `src/render/cpu.rs`: CPU backend (vello_cpu + SVG via resvg)
- `src/render/pipeline.rs`: `render_frame`, `render_frames`, `render_to_mp4`
- `src/render/encode_ffmpeg.rs`: `ffmpeg` encoder process wrapper
- `src/assets/store.rs` / `src/assets/decode.rs`: prepared asset storage + decoding
- `src/assets/media.rs` / `src/audio/mix.rs`: feature-gated media decode + audio mix/mux helpers
- `src/composition/layout.rs`: track layout resolver
- `src/bin/wavyte.rs`: the CLI

---

## Development

Minimum supported Rust version (MSRV): **Rust 1.93** (`edition = "2024"`).

Quality gate:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

---

## License

This project is licensed under **AGPL-3.0-only**. See `LICENSE`.
