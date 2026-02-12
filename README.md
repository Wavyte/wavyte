# wavyte (v0.3.0)

Wavyte is a Rust-first engine for programmatic video composition and rendering.

v0.3 is a full internal rewrite focused on production-grade determinism and hot-loop performance.

## Workspace crates

- `wavyte`: core engine + v0.3 JSON schema + session API
- `wavyte-cli`: CLI (binary name: `wavyte`)
- `wavyte-std`: higher-level helpers (presets / small JSON builders)
- `wavyte-bench`: benchmark harness

## Prerequisites

- Rust toolchain (see `rust-toolchain.toml`)
- `ffmpeg` on `PATH` for MP4 encoding (`wavyte-cli render`, `FfmpegSink`)
- `ffprobe` + `ffmpeg` on `PATH` for probing/decoding video/audio assets (enable `wavyte` feature `media-ffmpeg`)

```bash
ffmpeg -version
ffprobe -version
```

## Local verification (release-only)

This repo runs “CI” locally (no hosted CI for now):

```bash
bash scripts/verify_release.sh
```

## Quick start (CLI)

Render a single PNG frame:

```bash
cargo run -p wavyte-cli --release -- frame --in comp.json --frame 0 --out out.png
```

Render an MP4 video:

```bash
cargo run -p wavyte-cli --release -- render --in comp.json --out out.mp4
```

## Quick start (examples)

```bash
cargo run -p wavyte --release --example v03_render_png
cargo run -p wavyte --release --example v03_render_mp4
```

## Minimal v0.3 JSON composition

```json
{
  "version": "0.3",
  "canvas": { "width": 256, "height": 256 },
  "fps": { "num": 30, "den": 1 },
  "duration": 60,
  "assets": {
    "solid": { "solid_rect": { "color": "#ff3366" } }
  },
  "root": {
    "id": "root",
    "kind": { "leaf": { "asset": "solid" } },
    "range": [0, 60]
  }
}
```

## API sketch (v0.3)

```rust
use wavyte::{Composition, CpuBackendOpts, FrameIndex, FrameRange, InMemorySink, RenderSession, RenderSessionOpts};

let comp = Composition::from_path("comp.json")?;
let mut session = RenderSession::new(&comp, ".", RenderSessionOpts::default())?;

let frame0 = session.render_frame(FrameIndex(0), CpuBackendOpts::default())?;

let mut sink = InMemorySink::new();
session.render_range(
    FrameRange::new(FrameIndex(0), FrameIndex(comp.duration_frames()))?,
    CpuBackendOpts::default(),
    &mut sink,
)?;
# Ok::<(), wavyte::WavyteError>(())
```

## Design docs

- `wavyte_v03_proposal_final.md`: v0.3 authority spec
- `TASKS.md`: implementation checklist used to build v0.3 (release-only verification)
- `EXPLANATION.md`: background / deep dive (historical, may lag the v0.3 rewrite)

