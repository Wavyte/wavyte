# wavyte (v0.1.0)

Programmatic video composition and rendering in Rust.

Wavyte is a library-first engine that turns a **timeline composition** (JSON or a Rust builder DSL)
into pixels via a deterministic pipeline:

1. **Evaluate** the timeline at a frame index → visible clips in painter’s order
2. **Compile** into a backend-agnostic **RenderPlan**
3. **Render** the plan on the **CPU by default** (optional GPU backend)
4. Optionally **encode MP4** by streaming frames to the system `ffmpeg` binary

Wavyte v0.1.0 is deliberately scoped: it aims to be a solid compositing/rendering baseline that we
can extend into a production-grade product over time.

---

## Table of contents

- Getting started
  - Prerequisites
  - Run an example
  - Minimal composition JSON (copy/paste)
- CLI
- Library usage (pipeline and API map)
- Backends and features
- Assets (paths, caching, determinism)
- Rendering semantics (`RenderPlan`, premultiplied alpha)
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
```

MP4 example (requires `ffmpeg`):

```bash
cargo run --example render_to_mp4
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

- CPU (default): omit `--backend` or pass `--backend cpu`
- GPU: pass `--backend gpu` and build with `--features gpu`

Implementation details (useful for debugging):

- The CLI validates the composition before rendering.
- The CLI uses `FsAssetCache` rooted at the directory containing the `--in` JSON file, so relative
  asset paths resolve relative to the composition file.

---

## Library usage

Wavyte’s core units are:

- `Composition`: the timeline (assets + tracks + clips)
- `Evaluator`: resolves per-frame visibility and clip properties
- `RenderPlan`: backend-agnostic render IR
- `RenderBackend`: executes the IR into pixels
- `AssetCache`: isolates asset IO / decoding

The main convenience functions are in `src/pipeline.rs`:

- `render_frame(&Composition, FrameIndex, backend, assets) -> FrameRGBA`
- `render_frames(&Composition, FrameRange, backend, assets) -> impl Iterator<Item = FrameRGBA>`
- `render_to_mp4(&Composition, out_path, RenderToMp4Opts, backend, assets) -> WavyteResult<()>`

Backend creation:

```rust
let settings = wavyte::RenderSettings {
    clear_rgba: Some([18, 20, 28, 255]),
};
let mut backend = wavyte::create_backend(wavyte::BackendKind::Cpu, &settings)?;
```

Asset IO is isolated:

- Renderers never read from disk directly.
- All external assets are loaded/decoded through `AssetCache` (use `FsAssetCache` for local files).

For a guided tour of the public API, see the module map in “Project layout” below and the crate
documentation on docs.rs.

---

## Backends and features

### CPU backend (default)

Always available (no Cargo feature gate).

- Raster engine: `vello_cpu`
- SVG support: `usvg` parse + `resvg` rasterize (premultiplied RGBA8)

### GPU backend (`--features gpu`)

Optional. Enable it with:

```bash
cargo build --features gpu
```

GPU rendering uses:

- `vello` for scene building and rendering
- `wgpu` for device/queue/surface management and readback to RGBA8
- `vello_svg` to convert `usvg::Tree` into a `vello` scene segment

If you request `BackendKind::Gpu` without building with `--features gpu`,
backend creation fails with a clear error.

---

## Assets (paths, caching, determinism)

External asset sources (image/svg/text font) must be:

- relative paths (no leading `/`)
- OS-agnostic (backslashes normalized to `/`)
- free of `..`

Wavyte enforces these rules during validation and asset key normalization.

Caching:

- `AssetId` is stable and derived from the normalized asset key + parameters.
- `FsAssetCache` memoizes decoded/prepared assets so repeated frames do not re-decode.

Important v0.1.0 rule:

- Validation checks that asset sources are well-formed paths, but does not require that the files
  exist. IO errors are reported when an asset is actually loaded during rendering.

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

This separation is what lets CPU and GPU backends share a single compiler.

---

## MP4 encoding and `ffmpeg`

Wavyte encodes MP4 by invoking the system `ffmpeg` binary and streaming raw RGBA frames to stdin.
This is intentionally a **runtime prerequisite**, not a Cargo feature.

Where this lives:

- `src/encode_ffmpeg.rs`: `FfmpegEncoder` + config validation + process spawning
- `src/pipeline.rs`: orchestration via `render_to_mp4`

Behavior:

- If `ffmpeg` is not found on `PATH`, encoding returns an error (no silent fallback).
- Frames are rendered as premultiplied RGBA8. The encoder can “flatten” alpha over a background
  color (see `EncodeConfig` / `default_mp4_config`).

---

## Project layout (where to look)

- `src/model.rs`: `Composition`, assets, tracks, clips, validation rules
- `src/anim*.rs`: `Anim<T>` and helpers (keyframes, sequencing, easing)
- `src/eval.rs`: `Evaluator` and the evaluated graph (visibility + resolved transforms)
- `src/compile.rs`: backend-agnostic compiler into `RenderPlan`
- `src/render.rs`: backend trait + backend selection (`BackendKind`, `create_backend`)
- `src/render_cpu.rs`: CPU backend (vello_cpu + SVG via resvg)
- `src/render_vello.rs`: GPU backend (vello + wgpu) (only when built with `--features gpu`)
- `src/pipeline.rs`: `render_frame`, `render_frames`, `render_to_mp4`
- `src/encode_ffmpeg.rs`: `ffmpeg` encoder process wrapper
- `src/assets.rs` / `src/assets_decode.rs`: `AssetCache` + in-memory decoding
- `src/bin/wavyte.rs`: the CLI

---

## Development

Minimum supported Rust version (MSRV): **Rust 1.93** (`edition = "2024"`).

Quality gate:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo test --all-targets --features gpu
```

---

## License

This project is licensed under **AGPL-3.0-only**. See `LICENSE`.
