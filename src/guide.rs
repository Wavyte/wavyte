//! # Wavyte guide (v0.1.0)
//!
//! This module is a standalone, end-to-end walkthrough of Wavyte’s architecture and public API.
//! It is intentionally detailed so future phases (and external integrations) can build on a shared
//! mental model of what “a render” means in this codebase.
//!
//! If you are looking for copy/paste commands, start with the repository `README.md`.
//! If you are implementing new features, start here.
//!
//! ---
//!
//! ## Core concepts
//!
//! - [`Composition`](crate::Composition): the timeline (assets + tracks + clips) and global settings
//! - [`FrameIndex`](crate::FrameIndex): a 0-based frame index within the composition’s duration
//! - [`Evaluator`](crate::Evaluator): resolves what is visible at a frame and in what order
//! - [`RenderPlan`](crate::RenderPlan): backend-agnostic render IR for a single frame
//! - [`RenderBackend`](crate::RenderBackend): executes a plan into pixels
//! - [`FrameRGBA`](crate::FrameRGBA): the output pixels (RGBA8, premultiplied alpha)
//! - [`AssetCache`](crate::AssetCache): the only place external IO is allowed
//!
//! The rendering pipeline is explicitly staged:
//!
//! 1. Evaluate timeline visibility: [`Evaluator::eval_frame`](crate::Evaluator::eval_frame)
//! 2. Compile into passes: [`compile_frame`](crate::compile_frame)
//! 3. Execute passes: [`RenderBackend::render_plan`](crate::RenderBackend::render_plan)
//!
//! Convenience wrappers for step (1)+(2)+(3) live in:
//! - [`render_frame`](crate::render_frame)
//! - [`render_frames`](crate::render_frames)
//! - [`render_to_mp4`](crate::render_to_mp4)
//!
//! ---
//!
//! ## “No IO in the renderer” (and why)
//!
//! Wavyte wants evaluation/compilation/rendering to be deterministic, testable, and portable.
//! To do that, renderer code never reaches into the filesystem (or network).
//! Instead:
//!
//! - IO and decoding happen through [`AssetCache`](crate::AssetCache)
//! - Renderers consume **prepared** assets:
//!   - [`PreparedImage`](crate::PreparedImage) (premultiplied RGBA8)
//!   - [`PreparedSvg`](crate::PreparedSvg) (`usvg::Tree`)
//!   - [`PreparedText`](crate::PreparedText) (Parley layout + font bytes)
//!
//! The default implementation is [`FsAssetCache`](crate::FsAssetCache), which loads assets from a
//! root directory and memoizes prepared results.
//!
//! This design makes it straightforward to add a future asset cache that loads from:
//! - an in-memory store
//! - a content-addressed CAS
//! - a remote object store
//! without changing renderer logic.
//!
//! ---
//!
//! ## Premultiplied alpha (Wavyte’s pixel contract)
//!
//! Wavyte’s internal and output pixel convention is **premultiplied RGBA8**:
//!
//! - decoded images are premultiplied at ingest
//! - render backends output premultiplied pixels in [`FrameRGBA`](crate::FrameRGBA)
//! - CPU compositing and effects assume premultiplied alpha
//! - MP4 encoding may optionally flatten alpha over a background color
//!
//! If you integrate Wavyte with external compositors, this is the most important contract to
//! preserve. Treat `FrameRGBA.data` as premultiplied unless explicitly stated otherwise by the API.
//!
//! ---
//!
//! ## Building a composition (Rust DSL)
//!
//! JSON is supported via Serde, but the JSON representation is necessarily verbose because many
//! timeline fields are modeled as animated values ([`Anim<T>`](crate::Anim)).
//! For programmatic usage, prefer the builder DSL.
//!
//! The following example builds a minimal composition containing a single inlined path asset
//! (no external IO needed), then renders one frame on the CPU backend.
//!
//! ```rust,no_run
//! use wavyte::{
//!     Anim, Asset, BackendKind, Canvas, ClipBuilder, CompositionBuilder, Fps, FrameIndex,
//!     FrameRange, FsAssetCache, PathAsset, RenderSettings, TrackBuilder, Transform2D, Vec2,
//!     create_backend, render_frame,
//! };
//!
//! # fn main() -> wavyte::WavyteResult<()> {
//! let comp = CompositionBuilder::new(
//!     Fps::new(30, 1)?,
//!     Canvas { width: 512, height: 512 },
//!     FrameIndex(60),
//! )
//! .seed(1)
//! .asset(
//!     "rect",
//!     Asset::Path(PathAsset {
//!         svg_path_d: "M0,0 L120,0 L120,120 L0,120 Z".to_string(),
//!     }),
//! )?
//! .track(
//!     TrackBuilder::new("main")
//!         .clip(
//!             ClipBuilder::new(
//!                 "c0",
//!                 "rect",
//!                 FrameRange::new(FrameIndex(0), FrameIndex(60))?,
//!             )
//!             .transform(Anim::constant(Transform2D {
//!                 translate: Vec2::new(180.0, 180.0),
//!                 scale: Vec2::new(2.5, 2.5),
//!                 ..Transform2D::default()
//!             }))
//!             .build()?,
//!         )
//!         .build()?,
//! )
//! .build()?;
//!
//! let settings = RenderSettings {
//!     clear_rgba: Some([18, 20, 28, 255]),
//! };
//! let mut backend = create_backend(BackendKind::Cpu, &settings)?;
//! let mut assets = FsAssetCache::new("."); // only used for external assets (images/svg/text)
//!
//! let frame = render_frame(&comp, FrameIndex(0), backend.as_mut(), &mut assets)?;
//! assert_eq!(frame.width, 512);
//! assert_eq!(frame.height, 512);
//! assert!(frame.premultiplied);
//! assert_eq!(frame.data.len(), 512 * 512 * 4);
//! # Ok(())
//! # }
//! ```
//!
//! Notes:
//!
//! - [`Composition::validate`](crate::Composition::validate) is called by the builder.
//! - The example uses [`Anim::constant`](crate::Anim::constant) to avoid verbose keyframes.
//!
//! ---
//!
//! ## Asset paths and validation
//!
//! For assets that reference external files (`Image`, `Svg`, `Text` font sources), v0.1.0 enforces:
//!
//! - **relative** paths (no leading `/`)
//! - OS-agnostic separators (`\` normalized to `/`)
//! - no `..` components
//!
//! These checks happen during composition validation.
//!
//! Important: validation checks that the path is well-formed, but does not require that the file
//! exists. IO errors are surfaced when an asset is actually loaded through [`AssetCache`].
//!
//! ---
//!
//! ## Evaluation: from timeline to visible nodes
//!
//! [`Evaluator`](crate::Evaluator) converts a `Composition` into an [`EvaluatedGraph`](crate::EvaluatedGraph)
//! at a given frame index:
//!
//! - clips are filtered by their [`FrameRange`](crate::FrameRange)
//! - animated properties are sampled ([`Anim::sample`](crate::Anim::sample))
//! - transforms are resolved into a `kurbo::Affine` (re-exported as [`Affine`](crate::Affine))
//! - opacities are clamped into `[0, 1]`
//! - transitions are resolved into typed transition instances with progress
//!
//! The evaluated graph is painter’s-order: later nodes are drawn “on top”.
//!
//! ---
//!
//! ## Compilation: from nodes to `RenderPlan`
//!
//! The compiler takes visible nodes and produces a backend-agnostic plan:
//!
//! - explicit surfaces with known sizes/formats
//! - an ordered list of passes
//! - draw operations and composites expressed in terms of stable asset IDs
//!
//! In v0.1.0, the plan uses these pass types:
//!
//! - [`ScenePass`](crate::ScenePass)
//! - [`OffscreenPass`](crate::OffscreenPass) (currently used for blur)
//! - [`CompositePass`](crate::CompositePass) (`Over`, `Crossfade`, `Wipe`)
//!
//! ### Draw operations and coordinate conventions
//!
//! Each [`DrawOp`](crate::DrawOp) carries:
//!
//! - a [`Affine`](crate::Affine) transform
//! - an opacity factor in `[0, 1]`
//! - a blend mode (currently only [`BlendMode::Normal`](crate::BlendMode))
//! - an integer `z` used for ordering within a pass
//!
//! v0.1.0 draw ops:
//!
//! - `FillPath`:
//!   - the local coordinate space is the SVG path coordinates parsed into a [`BezPath`](crate::BezPath)
//!   - the op’s `transform` maps local path space into canvas space
//! - `Image`:
//!   - the local coordinate space is `[0,0]..[width,height]` in image pixels
//!   - the `transform` maps pixel space into canvas space
//! - `Svg`:
//!   - the asset is a `usvg::Tree` (vector)
//!   - v0.1.0: rasterized via `resvg` into a pixmap (premultiplied RGBA8), then drawn as an image
//!   - note: we intentionally use the same rasterization path on CPU and GPU to ensure SVG `<text>`
//!     correctness; a future version can add an optional vector SVG pipeline.
//! - `Text`:
//!   - the asset is a prepared Parley layout
//!   - glyph positioning originates in the Parley layout coordinate space; the op `transform`
//!     positions it in canvas space
//!
//! ### Effects and transitions (v0.1.0)
//!
//! v0.1.0 supports a small set of effects and transitions, chosen specifically to validate the
//! multi-pass architecture.
//!
//! - Effects:
//!   - inline effects (folded into op transform/opacity at compile time)
//!   - pass effects (implemented as an [`OffscreenPass`](crate::OffscreenPass))
//!   - currently implemented pass effect: blur
//! - Transitions:
//!   - crossfade
//!   - wipe (direction + soft edge)
//!
//! Transitions are compiled into [`CompositePass`](crate::CompositePass) operations rather than
//! being “baked” into draw op opacity, because they conceptually combine multiple rendered layers.
//!
//! Why this exists:
//!
//! - render backends can share the same compilation logic
//! - tests can validate compiler output without involving a renderer
//! - future effects/transitions can be expressed once in the IR
//!
//! ---
//!
//! ## Rendering: backends and `RenderBackend`
//!
//! A renderer implements [`RenderBackend`](crate::RenderBackend), which extends
//! [`PassBackend`](crate::PassBackend) (the trait that knows how to execute individual passes).
//!
//! Backend construction is done via:
//!
//! - [`BackendKind`](crate::BackendKind): `Cpu`
//! - [`create_backend`](crate::create_backend): returns `Box<dyn RenderBackend>`
//!
//! CPU backend (always available):
//!
//! - powered by `vello_cpu`
//! - SVG is supported by rasterizing `usvg::Tree` via `resvg` into an RGBA pixmap
//!
//! Wavyte v0.2 focuses on CPU rendering only.
//!
//! ---
//!
//! ## MP4 encoding: `ffmpeg` as a runtime prerequisite
//!
//! Wavyte intentionally does not ship a built-in MP4 encoder. Instead, it wraps the system `ffmpeg`
//! binary:
//!
//! - [`FfmpegEncoder`](crate::FfmpegEncoder) spawns `ffmpeg` and streams raw frames to stdin
//! - [`render_to_mp4`](crate::render_to_mp4) orchestrates frame rendering and feeding the encoder
//!
//! `ffmpeg` must be installed and on `PATH`. If it is not available, encoding returns a structured
//! error; there is no silent fallback.
//!
//! The encoder configuration surface is:
//!
//! - [`EncodeConfig`](crate::EncodeConfig)
//! - [`default_mp4_config`](crate::default_mp4_config)
//!
