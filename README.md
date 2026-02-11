# wavyte (v0.2.1)

Wavyte is a Rust-first engine for programmatic video composition and rendering.

The long-term goal is simple: make automated, high-volume video generation feel like software engineering,
not timeline clicking.

If you are building short-form content pipelines driven by AI-generated text/audio/images/video,
you need a composition engine that is deterministic, scriptable, and fast. Wavyte is being built for
that role.

## Try it in 2 minutes

If Rust and `ffmpeg` are installed, run:

```bash
cargo run -p wavyte-core --example render_remotion_hello_world_mp4
```

You should get a rendered MP4 in the repo `assets/` directory.

## The problem we are solving

Video creation demand has exploded, especially for short-form formats, while asset generation has become
increasingly automated (LLMs, TTS, image generators, video generators).

Most teams now need to generate or personalize large amounts of content across many variants.
That workflow does not scale well when composition logic lives in a GUI timeline. It needs code-level
control, reproducibility, and performance.

Wavyte exists to be the composition driver in those pipelines.

## Why Wavyte

Tools like Remotion and MoviePy have proven how powerful programmatic video can be.
Wavyte aims to become a strong alternative in a Rust-native stack with:

- deterministic evaluate -> compile -> render architecture,
- explicit render IR (`RenderPlan`) for backend portability,
- IO-frontloaded asset preparation (`PreparedAssetStore`),
- performance-oriented runtime behavior for large batch workloads.

Wavyte is not yet feature-complete versus mature ecosystems. The current focus is building a clean,
extensible core that can support long-term ergonomics and multi-language consumers.

## Where we want to win

- Better ergonomics for automation-heavy workflows
- Better determinism and reliability for batch generation
- Better performance profile for high-volume rendering
- Better long-term architecture for cross-language and GUI consumers

In short: become a practical, production-grade alternative to Remotion and MoviePy for teams that need
programmatic composition at scale.

## Vision

Wavyte is being shaped as the engine beneath multiple downstream products:

1. `wavyte-core` (today): the rendering/composition engine.
2. `wavyte-std` (planned): high-level ergonomic abstractions for layouts, animation chains, effects,
   and reusable visual components.
3. `wavyte-py` and `wavyte-ts` (planned): Python and TypeScript bindings for broader ecosystem use.
4. `wavyte-stitch` (planned): a service-backed GUI editor for no-code short-form composition.

That is why JSON-driven composition contracts and strong internal system consistency are core priorities.

## Who this is for (today)

- Engineers building automated social/video generation pipelines
- Teams comfortable with code-first composition
- Users who want deterministic rendering behavior and explicit architecture boundaries

If you are looking for a polished no-code editor today, that is not this repo yet.

Another big note: Since we are consolidating the core engine, it is currently quite verbose and raw to work with.

In the next version (v0.3), we will taking a major effort on actually making working with Wavyte joyous and ergonomic.

## What works today (v0.2.1)

- Workspace crates:
  - `wavyte-core` (library crate name: `wavyte`)
  - `wavyte-cli` (binary: `wavyte`)
  - `bench` (standalone benchmark harness)
- CPU rendering backend (`vello_cpu`) with premultiplied RGBA semantics
- Composition model + Rust DSL builders + JSON serde
- Track layout primitives: `Absolute`, `HStack`, `VStack`, `Grid`, `Center`
- Effects/transitions pipeline:
  - transitions: `Crossfade`, `Wipe`
  - effects: inline opacity/transform + pass blur
- Chunked parallel rendering with optional static-frame elision
- Optional media decode/probe and audio mix/mux via `media-ffmpeg`
- MP4 encoding through system `ffmpeg`

## Architecture at a glance

Wavyte runs a staged, deterministic pipeline:

1. Evaluate timeline state for a frame (`Evaluator`).
2. Compile evaluated nodes into backend-agnostic `RenderPlan` IR.
3. Execute render passes on a backend (`RenderBackend`, currently CPU).
4. Optionally stream frames into `ffmpeg` for MP4 output.

For a full end-to-end technical walkthrough, read `EXPLANATION.md`.

## Install and prerequisites

Rust toolchain:

```bash
rustc --version
```

MP4 render/encode path requires `ffmpeg` + `ffprobe` on `PATH`:

```bash
ffmpeg -version
ffprobe -version
```

## Quick start

Run core examples:

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

Examples write outputs under repo-local `assets/`.

Choose your starting path:

- Want to understand architecture deeply: read `EXPLANATION.md`.
- Want to ship quickly from JSON compositions: use `wavyte-cli` commands below.
- Want Rust-native control: use the library APIs (`render_frame`, `render_to_mp4_with_stats`).

## CLI usage

Render one PNG frame from JSON:

```bash
cargo run -p wavyte-cli --bin wavyte -- frame --in comp.json --frame 0 --out out.png
```

Render MP4 from JSON:

```bash
cargo run -p wavyte-cli --bin wavyte -- render --in comp.json --out out.mp4
```

Useful diagnostics:

- `--dump-fonts`: resolved text family + font SHA-256
- `--dump-svg-fonts`: SVG text node count + loaded SVG font face count

## Minimal JSON composition

```json
{
  "fps": { "num": 30, "den": 1 },
  "canvas": { "width": 512, "height": 512 },
  "duration": 60,
  "assets": {
    "rect": { "Path": { "svg_path_d": "M0,0 L120,0 L120,120 L0,120 Z" } }
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

Render it:

```bash
cargo run -p wavyte-cli --bin wavyte -- frame --in comp.json --frame 0 --out out.png
```

## API entry points

Core public APIs (see `wavyte-core/src/render/pipeline.rs`):

- `render_frame(...) -> FrameRGBA`
- `render_frames_with_stats(...) -> (Vec<FrameRGBA>, RenderStats)`
- `render_to_mp4_with_stats(...) -> RenderStats`

Backend creation:

```rust
let settings = wavyte::RenderSettings {
    clear_rgba: Some([18, 20, 28, 255]),
};
let mut backend = wavyte::create_backend(wavyte::BackendKind::Cpu, &settings)?;
```

## Current constraints to know

- Render backend is CPU-first today.
- MP4 path requires system `ffmpeg`.
- Current MP4 API expects integer FPS (`fps.den == 1`) and even dimensions.
- Public surface is still evolving as groundwork for `wavyte-std`, bindings, and GUI service.

## Project layout

- `wavyte-core/src/foundation/`: errors, core types, shared math/hash helpers
- `wavyte-core/src/animation/`: animation/easing/procedural/operators
- `wavyte-core/src/transform/`: linear, affine, and non-linear helpers
- `wavyte-core/src/effects/`: effect parse/normalize + blur/composite/transitions
- `wavyte-core/src/layout/`: layout solver
- `wavyte-core/src/composition/`: model + DSL builders
- `wavyte-core/src/eval/`: evaluator/frame graph
- `wavyte-core/src/compile/`: render IR/plan + fingerprinting
- `wavyte-core/src/render/`: backends, pass execution, CPU impl, pipeline
- `wavyte-core/src/audio/`: manifest and mixer
- `wavyte-core/src/encode/`: ffmpeg encoder wrapper
- `wavyte-core/src/assets/`: prepare/decode/media/raster helpers
- `wavyte-cli/src/main.rs`: CLI entrypoint
- `bench/src/main.rs`: benchmark harness
- `EXPLANATION.md`: exhaustive architecture walkthrough

## Release gate

MSRV: Rust `1.93` (edition `2024`).

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features --release
cargo test -p wavyte-bench --release
```

## License

AGPL-3.0-only (`LICENSE`).
