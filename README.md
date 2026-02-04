# wavyte

Fast and ergonomic programmatic video generation (Rust).

This repository is currently implementing **Wavyte v0.1.0**. As of **end of Phase 6**, the crate can:

- Build a `Composition` (JSON or DSL)
- Evaluate a frame into an ordered list of visible clip nodes
- Compile that into a backend-agnostic `RenderPlan`
- Render passes (multi-surface) with:
  - CPU and GPU compositing (`Over`, `Crossfade`, `Wipe`)
  - Blur as an offscreen pass (CPU + GPU)
- Render a single frame into **premultiplied RGBA8** pixels via:
  - **CPU backend** (default): `vello_cpu` + (for SVG) `resvg`
  - **GPU backend** (opt-in): `vello` + `wgpu`, with readback to RGBA8
- Encode MP4 videos by invoking the system `ffmpeg` binary (must be installed and on `PATH`)
- A small CLI (`wavyte`) for `frame` and `render`

## Status: end of Phase 6

### What’s implemented

**Core data model**

- `model::Composition` with assets + tracks + clips (`src/model.rs`)
- `Anim<T>` primitives (constant, keyframes, sequencing) (`src/anim*.rs`)

**Evaluation**

- `Evaluator::eval_frame(&Composition, FrameIndex) -> EvaluatedGraph` (`src/eval.rs`)
- Produces painter’s-order nodes with:
  - resolved transform (as `kurbo::Affine`)
  - resolved opacity (clamped)
  - resolved blend mode (currently only `Normal`)
  - resolved transitions (typed + params)

**Asset system (no renderer IO)**

- `AssetCache` trait (`src/assets.rs`)
  - All filesystem access is isolated behind `FsAssetCache` (the renderer never reads files)
- `AssetId` is **stable** (FNV-1a over normalized key + params)
  - Asset paths must be **relative**, OS-agnostic, and must not contain `..`
- `PreparedAsset` variants:
  - `PreparedImage` (premultiplied RGBA8)
  - `PreparedSvg` (`usvg::Tree`)
  - `PreparedText` (`parley::Layout` + font bytes, so renderers never do font IO)

**Compilation**

- `compile_frame(&Composition, &EvaluatedGraph, &mut dyn AssetCache) -> RenderPlan` (`src/compile.rs`)
- `RenderPlan` is a per-frame list of passes over multiple surfaces:
  - `Pass::Scene` (draw clip content into a surface)
  - `Pass::Offscreen` (effects like blur into a new surface)
  - `Pass::Composite` (layer surfaces into the final)
- Draw ops supported in v0.1.0:
  - `FillPath` (from `PathAsset` SVG path `d`)
  - `Image` (by `AssetId`)
  - `Svg` (by `AssetId`)
  - `Text` (by `AssetId`)

**Rendering**

- Unified interface: `RenderBackend::render_plan(&RenderPlan, &mut dyn AssetCache) -> FrameRGBA` (`src/render.rs`)
- `FrameRGBA` is `Vec<u8>` RGBA8, **premultiplied**
- Orchestration: `render_frame(&Composition, FrameIndex, backend, assets)` (`src/pipeline.rs`)
- MP4 output:
  - `render_to_mp4(&Composition, out_path, opts, backend, assets)` (`src/pipeline.rs`)
  - Uses `FfmpegEncoder` (`src/encode_ffmpeg.rs`) which spawns `ffmpeg` and streams raw RGBA frames to stdin.
    If `ffmpeg` is not available, this returns an error.

## Design principles (important for Phases 5–6)

- **No unsafe**: the crate forbids `unsafe`.
- **Deterministic by default**:
  - `Composition.seed` exists for deterministic procedural sources later.
  - Current tests assert stability of evaluation and rendering for simple scenes.
- **No IO in evaluation/compile/render**:
  - Only `FsAssetCache` reads from disk.
  - Renderers only consume already-prepared `PreparedAsset`s.
- **Premultiplied RGBA8 everywhere**:
  - Images are decoded and premultiplied at ingest.
  - Render backends output premultiplied RGBA8, matching `vello_cpu::Pixmap` semantics.

## Features

Wavyte keeps GPU dependencies optional while still working out-of-box.

- CPU backend (`BackendKind::Cpu`) is always available (no Cargo feature gate).
- The `wavyte` CLI binary is always built (no Cargo feature gate).
- `gpu`
  - Enables `BackendKind::Gpu` using `vello` + `wgpu` with readback to RGBA8.
  - Note: `wgpu` is built with `default-features = false`; the `gpu` feature wires common native backends
    (DX12/Metal/Vulkan/GLES) so it works on Linux/Windows/macOS.

`ffmpeg` is a runtime prerequisite for MP4 encoding (not a Cargo feature).

## Backends

### CPU backend (default)

Implementation: `src/render_cpu.rs`

Powered by:

- `vello_cpu` (CPU 2D renderer): https://docs.rs/vello_cpu
- `resvg` (SVG renderer into a pixmap): https://docs.rs/resvg/latest/resvg/fn.render.html

Supported ops (Phase 4):

- Path fill (`DrawOp::FillPath`)
- Image draw (`DrawOp::Image`)
- Text draw (`DrawOp::Text`)
- SVG draw (`DrawOp::Svg`)
  - Implemented by rasterizing `usvg::Tree` via `resvg` into a premultiplied RGBA8 pixmap,
    then drawing that pixmap as an image.
  - Current behavior rasterizes at the SVG’s intrinsic size (`tree.size()`), then applies the op’s transform.
    (In Phases 5–6 we may want “render at output scale” caching to avoid upscaling blur.)

Notes/constraints:

- `vello_cpu` currently uses `u16` dimensions for its pixmap; very large canvases will error.

### GPU backend (opt-in)

Implementation: `src/render_vello.rs`

Powered by:

- `vello` (GPU renderer): https://docs.rs/vello
- `wgpu` (graphics API): https://docs.rs/wgpu
- `vello_svg` (SVG→Scene adapter): https://docs.rs/vello_svg

Key details:

- Uses `Renderer::render_to_texture` to render into an offscreen RGBA8 texture.
- **Must call `Scene::reset()` each frame** (Vello explicitly does not clear the scene). See:
  https://docs.rs/vello/latest/vello/struct.Scene.html
- Reads pixels back via `wgpu` buffer mapping and returns premultiplied RGBA8.
- If no GPU adapter exists (e.g. headless CI), the backend returns:
  `"no gpu adapter available"` (tests/examples treat this as a skip).

## How the pipeline fits together

High-level flow for rendering one frame:

1. `Evaluator::eval_frame(comp, frame)` produces an `EvaluatedGraph`
2. `compile_frame(comp, &eval, assets)` produces a `RenderPlan`
3. `backend.render_plan(&plan, assets)` produces `FrameRGBA`

This split is intentional for Phases 5–6:

- Phase 5 will expand `RenderPlan` into multiple passes and richer ops (effects/transitions)
- Phase 6 will consume `FrameRGBA` frames for encoding/output

## Code map (Phase 4)

If you’re implementing Phases 5–6, this is the fastest “where should I change things?” index.

- `src/model.rs`: `Composition` / `Asset` / `Clip` / validation rules
- `src/anim*.rs`: animation + easing + procedural determinism helpers
- `src/eval.rs`: resolves `Composition + FrameIndex` into `EvaluatedGraph` (painter’s order)
- `src/assets.rs`: `AssetCache`, `FsAssetCache`, stable `AssetId`, `PreparedAsset` types
- `src/assets_decode.rs`: in-memory decoders (image premultiply, SVG parse)
- `src/compile.rs`: backend-agnostic compiler `EvaluatedGraph -> RenderPlan`
- `src/render.rs`: `FrameRGBA`, `RenderBackend`, backend selection (`create_backend`)
- `src/render_cpu.rs`: CPU backend (`vello_cpu` + CPU SVG via `resvg`)
- `src/render_vello.rs`: GPU backend (`vello` + `wgpu` + readback)
- `src/pipeline.rs`: orchestration (`eval -> compile -> render_plan`)
- `src/lib.rs`: public API re-exports (the “user-facing surface”)

## RenderPlan semantics (Phase 4)

`RenderPlan` (`src/compile.rs`) is the bridge between “timeline evaluation” and “pixel backends”.

### Passes

- Phase 4 only has `Pass::Scene(ScenePass)`.
- `ScenePass.clear` exists in the IR but is not used yet; clearing is currently done via
  `RenderSettings.clear_rgba` (both CPU and GPU backends honor this).

### Draw operations and coordinate conventions

All draw ops carry a `kurbo::Affine` transform (re-exported as `wavyte::Affine`).

- `FillPath`
  - The local coordinate space is the `BezPath` coordinates (parsed from SVG path `d`).
  - Phase 4 uses a fixed fill color (white) and multiplies it by opacity.
- `Image`
  - The renderer loads `PreparedImage` by `AssetId`.
  - The local coordinate space is a rectangle `[0,0]..[width,height]` in image pixels.
  - The op’s `transform` maps that local space into canvas space.
- `Svg`
  - The renderer loads `PreparedSvg` (`usvg::Tree`) by `AssetId`.
  - GPU: converts `usvg::Tree` to a `vello::Scene` segment via `vello_svg`.
  - CPU: rasterizes the `usvg::Tree` via `resvg` at the SVG’s intrinsic size, then draws as an image.
- `Text`
  - The renderer loads `PreparedText` (Parley layout + font bytes) by `AssetId`.
  - Both backends iterate Parley glyph runs and emit glyph draws.

## Asset path rules (Phase 3)

External asset sources (`ImageAsset.source`, `SvgAsset.source`, `TextAsset.font_source`, etc.) must be:

- Relative paths (no leading `/`)
- OS-agnostic (backslashes normalized to `/`)
- Free of `..`

These rules are enforced during `Composition::validate()` and when constructing `AssetKey`s for `AssetId`.

## Examples

- `cargo run --example build_dsl_and_dump_json`
- `cargo run --example eval_frames`
- Render to PNG (writes into untracked `assets/`):
  - Crossfade: `cargo run --example render_crossfade_png`
  - Blur: `cargo run --example render_blur_png`
- Render to MP4 (writes into untracked `assets/`, requires `ffmpeg` on `PATH`):
  - `cargo run --example render_to_mp4`
- Render one frame (writes `target/render_one_frame.png`):
  - CPU (default): `cargo run --example render_one_frame`
  - GPU: `cargo run --example render_one_frame --features gpu -- gpu`

The `assets/` directory is intentionally untracked; examples discover assets at runtime and will
gracefully omit image/text/svg layers if files are missing.

## Tests

Run the full quality gate (required before commits in this repo):

- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets`
- `cargo test --all-targets --features gpu`

Notes:

- CPU renderer tests are not feature-gated; they run under the default test invocation.
- GPU tests skip if the environment has no adapter (they treat `"no gpu adapter available"` as a skip).

## Known limitations (v0.1.0)

- Only `BlendMode::Normal` is implemented.
- Only path fill is implemented (no stroke yet).
- Video/audio assets exist in the model but are not renderable yet (future).
- CPU SVG is rasterized (via `resvg`); it is not a vector path conversion.
- MP4 output requires `ffmpeg` to be installed and on `PATH`.

## Where Phase 5–6 work will land

This section is meant as a navigation map for upcoming phases.

**Phase 5 (passes/effects/transitions)**

- Add richer IR:
  - Extend `src/compile.rs` with multiple `Pass`es (offscreen render targets) and effect ops.
  - Keep `compile` backend-agnostic (no `wgpu`/`vello` dependencies).
- Implement effects:
  - CPU path: likely via `vello_cpu` operations or CPU-side pixel filters.
  - GPU path: likely via additional scenes, layers, and/or compute/shader passes.
- Extend tests:
  - deterministic hashes for scene graphs / passes
  - parity tests with tolerances for non-trivial effects

**Phase 6 (output/encoding)**

- Introduce a frame sink / encoder abstraction that consumes `FrameRGBA`.
- Likely add CLI entrypoints and encode to sequences / mp4 (exact approach TBD).
