# Wavyte v0.2.1 - EXPLANATION.md (end-to-end codebase walkthrough)

This document is the authoritative v0.2.1 code walkthrough for the current `wavyte-core/src/` tree.

Scope:
- Explain the runtime architecture end-to-end.
- Explain every `wavyte-core/src/` file and the purpose of each struct/enum/impl/function group.
- Explain how data flows through evaluate -> compile -> render -> encode, and where audio/layout/media fit.
- Provide an auditable symbol appendix generated from the current tree.

Non-goals:
- This is not a replacement for rustdoc API pages.
- This is not a proposal doc for future versions.

Version/context snapshot:
- Workspace crates: `wavyte-core`, `wavyte-cli`, `wavyte-bench`
- Library crate name (from `wavyte-core`): `wavyte` `0.2.1`
- Rust: edition `2024`, `rust-version = 1.93`
- Backend in-tree: CPU (`vello_cpu`)
- Optional media decode: feature `media-ffmpeg`
- Encoding: system `ffmpeg` process (no linked ffmpeg C API)

## 1. System mental model

At runtime, Wavyte is a staged pipeline with deterministic boundaries:

1. Composition model (`composition/model.rs`)
- User timeline data: assets, tracks, clips, animated props, effects, transitions.
- Validation happens here.

2. Asset preparation (`assets/store.rs` + `assets/*`)
- External bytes and parse/decode work are front-loaded into immutable `PreparedAssetStore`.
- Renderers are intentionally IO-free.

3. Layout + evaluation (`layout/solver.rs` + `eval/evaluator.rs`)
- Track-level layout computes offsets from intrinsic asset dimensions.
- Frame evaluation produces visible nodes with sampled anims and resolved transitions.

4. Compilation (`compile/plan.rs`)
- Evaluated graph -> backend-agnostic `RenderPlan` (surfaces + passes + ops).

5. Pass execution (`render/passes.rs` + `render/cpu.rs`)
- A backend executes scene/offscreen/composite passes to produce a premultiplied frame.

6. Batch/pipeline APIs (`render/pipeline.rs`)
- Single-frame, frame-range, and MP4 rendering.
- Optional chunked parallel rendering with static-frame elision via fingerprinting.

7. Encoding (`encode/ffmpeg.rs`)
- Stream RGBA frames into `ffmpeg` stdin and write MP4.
- Optional mixed audio is piped as f32le side input.

### 1.1 ASCII visual atlas

This section is intentionally redundant with later prose. Its goal is fast orientation.

#### 1.1.1 End-to-end control/data sequence

```text
                           +----------------------------------+
                           | Composition (validated timeline) |
                           +-----------------+----------------+
                                             |
                                             | PreparedAssetStore::prepare(root)
                                             v
+--------------------+            +---------+---------+
| External IO/Decode |----------->| PreparedAssetStore|
| image/svg/text/video/audio      | (immutable assets)|
+--------------------+            +---------+---------+
                                             |
                                             | frame F
                                             v
                                +------------+-------------+
                                | resolve_layout_offsets() |
                                +------------+-------------+
                                             |
                                             v
                                 +-----------+-----------+
                                 | Evaluator::eval_frame |
                                 | -> EvaluatedGraph     |
                                 +-----------+-----------+
                                             |
                                             v
                                 +-----------+-----------+
                                 | compile_frame()       |
                                 | -> RenderPlan         |
                                 +-----------+-----------+
                                             |
                                             v
                                 +-----------+-----------+
                                 | execute_plan()        |
                                 | on RenderBackend      |
                                 +-----------+-----------+
                                             |
                                             v
                                      FrameRGBA (premul)
                                             |
                               +-------------+--------------+
                               |                            |
                               v                            v
                        caller consumes                FfmpegEncoder
                                                       (optional MP4)
```

#### 1.1.2 RenderPlan shape (single frame)

```text
RenderPlan
  canvas: WxH
  surfaces:
    [0] final canvas target (RGBA premul)
    [1] layer A scene output
    [2] layer A post-fx output (if any)
    [3] layer B scene output
    ...
  passes (ordered):
    Scene(target=1, ops=[...])
    Offscreen(input=1, output=2, fx=Blur{...})     # optional
    Scene(target=3, ops=[...])
    ...
    Composite(target=0, ops=[Over/Crossfade/Wipe...])
  final_surface: 0
```

#### 1.1.3 Composition model hierarchy

```text
Composition
├── fps, canvas, duration, seed
├── assets: BTreeMap<String, Asset>
│   ├── Text { text, font_source, size_px, ... }
│   ├── Svg  { source }
│   ├── Path { svg_path_d }
│   ├── Image{ source }
│   ├── Video{ source, trim/rate/volume/fades/mute }
│   └── Audio{ source, trim/rate/volume/fades/mute }
└── tracks: Vec<Track>
    └── Track
        ├── z_base
        ├── layout_* (mode/gap/padding/alignment/grid cols)
        └── clips: Vec<Clip>
            └── Clip
                ├── id, asset key, range[start,end)
                ├── props: transform Anim<Transform2D>, opacity Anim<f64>, blend
                ├── effects: Vec<EffectInstance>
                └── transitions: in/out (optional)
```

#### 1.1.4 Evaluation ordering and z semantics

```text
Visible clips at frame F
   |
   | each clip -> EvaluatedClipNode { z = track.z_base + clip.z_offset, ... }
   v
Sort key tuple:
   (z, track_index, clip.range.start, clip_id)
   ^
   | stable painter order
   +-- lower keys are composited first, later keys are visually on top
```

#### 1.1.5 Layout offset injection point

```text
Track layout solver
  + intrinsic sizes from PreparedAssetStore
  + track layout mode + alignment + padding + gap
  -> per-track, per-clip Vec2 offset

eval_clip transform composition:
  final_affine = Translate(layout_offset) * sampled_clip_transform
```

#### 1.1.6 CPU backend execution/caches

```text
CpuBackend
├── surfaces: HashMap<SurfaceId, CpuSurface> (reused across frames/chunks)
├── image_cache: AssetId -> vello_cpu::Image
├── svg_cache: (AssetId,width,height) -> vello_cpu::Image
├── font_cache: AssetId -> FontData
└── video_decoders: AssetId -> VideoFrameDecoder
    ├── frame cache + LRU keyed by rounded source milliseconds
    ├── cache capacity from `WAVYTE_VIDEO_CACHE_CAPACITY` (default 64)
    └── decode prefetch window from `WAVYTE_VIDEO_PREFETCH_FRAMES` (default 12)
```

#### 1.1.7 Parallel chunk rendering with optional static-frame elision

```text
FrameRange [start,end)
   |
   +-- split into chunks (chunk_size)
          |
          +-- for each chunk:
                eval all frames
                |
                +-- if static_frame_elision=off:
                |      unique_indices = all frames
                |      (no fingerprint computation)
                |
                +-- if static_frame_elision=on:
                       compute fingerprint per evaluated frame
                       unique_indices = first occurrence per fingerprint
                       frame_to_unique maps each timeline frame to unique slot
                |
                +-- rayon pool renders unique frames in parallel
                |
                +-- `render_frames*`: reconstruct full ordered frame vector (clone/move reuse)
                +-- `render_to_mp4*`: stream by `frame_to_unique` map without cloning duplicates
```

#### 1.1.8 Audio and MP4 orchestration flow

```text
render_to_mp4_with_stats()
  |
  +-- comp.validate() once
  +-- resolve_layout_offsets() once
  |
  +-- build_audio_manifest(range)
  |      includes AudioAsset + VideoAsset audio tracks
  |
  +-- mix_manifest() -> interleaved f32 stereo
  |
  +-- write temp .f32le (guarded temp file)
  |
  +-- spawn ffmpeg:
  |      input #0: raw RGBA frame stream (stdin)
  |      input #1: optional f32le audio file
  |      output  : MP4 (libx264 + yuv420p [+ aac if audio])
  |
  +-- render chunk-by-chunk:
  |      sequential: shared backend + rolling CompileCache
  |      parallel: worker-local backend + worker-local CompileCache
  +-- flatten premul alpha over bg color
  +-- stream frame bytes to ffmpeg stdin
  +-- finish() and remove temp file
```

## 2. Module map and dependency shape

`wavyte-core/src/lib.rs` wires modules by domain folders:
- `foundation/*`: core primitives, shared error type, and shared math/hash helpers.
- `animation/*`: animation sampling model, easing, ops, procedural sources.
- `transform/*`: linear/affine/non-linear transform helpers.
- `composition/*`: timeline model and DSL builders.
- `layout/*`: track layout resolution.
- `eval/*`: per-frame evaluation.
- `compile/*`: render IR/plan compiler and fingerprinting.
- `effects/*`: transitions, effect parse/normalize, blur/composite kernels.
- `assets/*`: decode/raster/media helpers and immutable prepared store.
- `render/*`: pass runner, backend abstraction, CPU backend, pipeline orchestration.
- `audio/*`: audio manifest construction and mixing.
- `encode/*`: ffmpeg encoder wrapper.
- `wavyte-cli/src/main.rs`: CLI.
- `wavyte-core/src/guide.rs`: crate-level high-level guide.

The library crate is `#![forbid(unsafe_code)]`.
Module wiring uses idiomatic per-folder `mod.rs` files (no `#[path = ...]` indirection).

Dependency direction (coarse, conceptual):

```text
foundation/*  <---------------------------------------------------+
   ^                                                           |
   |                                                           |
animation/* --> composition/model+dsl + layout + eval --------+--> compile + render/pipeline
transform/* ------------------------------^                    |
   ^                                                           |          |
   |                                                           |          v
   +---------------------------- assets/* ---------------------+     render/backend+passes+cpu
                                 |                             |
                                 +--> audio/mix --------------+
                                 |
                                 +--> media decode/probe (feature-gated)

render/pipeline --> encode/ffmpeg --> system ffmpeg process
wavyte-cli (CLI) depends on public lib surface
```

Compile-time boundary summary:
- Pure data/math path: `foundation`, `animation`, `transform`, most of `composition/layout/eval`, `compile`.
- IO/decode boundary: `assets/store`, `assets/media`, `audio/mix` temp-file write, encoder process spawn.
- Backend execution boundary: `render/passes` trait + `render/cpu` implementation.

## 3. Global invariants and contracts

1. Premultiplied RGBA contract
- Internal frame data is premultiplied RGBA8.
- Composite and blur code assumes premultiplied semantics.
- Encoder can flatten over opaque background.

2. IO isolation
- Renderer paths (`compile/render/passes/cpu`) do not touch filesystem.
- IO/decode is concentrated in `PreparedAssetStore::prepare` and media helpers.

3. Determinism
- Asset IDs are stable hashes of normalized keys.
- Evaluation is deterministic for same composition/frame/seed.
- Parallel rendering preserves frame order and can elide repeated fingerprints.

4. Validation first
- `Composition::validate` enforces path/media/layout/timeline invariants.
- Render APIs validate once at API boundary and then use unchecked per-frame evaluation internally.

## 4. File-by-file walkthrough (`wavyte-core/src/` + `wavyte-cli/src/`)

### 4.1 `wavyte-core/src/foundation/error.rs`
Purpose:
- Define crate-wide error and result aliases.

Types:
- `type WavyteResult<T> = Result<T, WavyteError>`.
- `enum WavyteError`:
  - `Validation(String)`
  - `Animation(String)`
  - `Evaluation(String)`
  - `Serde(String)`
  - `Other(anyhow::Error)`

Impl/functions:
- Constructors: `validation`, `animation`, `evaluation`, `serde`.
- `thiserror::Error` derive provides display formatting.

Tests:
- Prefix stability and wrapped-source propagation.

### 4.2 `wavyte-core/src/foundation/core.rs`
Purpose:
- Shared geometry/time/pixel primitives.

Re-exports:
- `kurbo::{Affine, BezPath, Point, Rect, Vec2}`.

Types:
- `FrameIndex(u64)`.
- `FrameRange { start, end }` (end exclusive).
- `Fps { num, den }`.
- `Canvas { width, height }`.
- `Rgba8Premul { r, g, b, a }`.
- `Transform2D { translate, rotation_rad, scale, anchor }`.

Functions/impl:
- `FrameRange::new`, `len_frames`, `is_empty`, `contains`, `clamp`, `shift`.
- `Fps::new`, `as_f64`, `frame_duration_secs`, `frames_to_secs`, `secs_to_frames_floor`.
- `Rgba8Premul::transparent`, `from_straight_rgba`.
- `Transform2D::to_affine` (translate * anchor * rotate * scale * unanchor).

### 4.2.1 `wavyte-core/src/foundation/math.rs`
Purpose:
- Shared low-level math/hash helpers used by multiple modules.

Types/functions:
- `Fnv1a64` seeded hasher used for deterministic IDs/fingerprints.
- `mul_div255_u16` and `mul_div255_u8` fixed-point helpers for premultiplied pixel math.

### 4.3 `wavyte-core/src/animation/ease.rs`
Purpose:
- Easing curves used by keyframe interpolation and transitions.

Types:
- `enum Ease`: `Linear`, `InQuad`, `OutQuad`, `InOutQuad`, `InCubic`, `OutCubic`, `InOutCubic`.

Functions:
- `Ease::apply(t)` clamps `t` to `[0,1]` and applies selected curve.

### 4.4 `wavyte-core/src/animation/proc.rs`
Purpose:
- Procedural animation source family.

Types:
- `Procedural<T> { kind, _marker }`.
- `trait ProcValue` with `from_procedural`.
- `ProceduralKind`:
  - `Scalar(ProcScalar)`
  - `Vec2 { x: ProcScalar, y: ProcScalar }`
- `ProcScalar`:
  - `Sine`, `Noise1D`, `Envelope`, `Spring`
- `Rng64` deterministic SplitMix64 helper.

Functions/impl:
- `Procedural::new`, `Procedural::sample`.
- `Rng64::new`, `next_u64`, `next_f64_01`.
- Internal: `noise01`, `sample_scalar`.
- `ProcValue` impls:
  - `f64`, `f32`, `Vec2` supported.
  - `Transform2D`, `Rgba8Premul` currently rejected with animation error.

Note:
- Some error strings still mention `v0.1` in source text, but behavior is current.

### 4.5 `wavyte-core/src/animation/anim.rs`
Purpose:
- Core typed animation model and expression runtime.

Types:
- `SampleCtx { frame, fps, clip_local, seed }`.
- `trait Lerp` and impls for `f64`, `f32`, `Vec2`, `Transform2D`, `Rgba8Premul`.
- `Anim<T>`:
  - `Keyframes(Keyframes<T>)`
  - `Procedural(Procedural<T>)`
  - `Expr(Expr<T>)`
- `Keyframes<T> { keys, mode, default }`.
- `Keyframe<T> { frame, value, ease }`.
- `InterpMode`: `Hold | Linear`.
- `Expr<T>`:
  - `Delay`, `Speed`, `Reverse`, `Loop`, `Mix`
- `LoopMode`: `Repeat | PingPong`.

Functions/impl:
- `Anim::constant`, `Anim::sample`, `Anim::validate`.
- `Keyframes::validate`, `Keyframes::sample`.
- `Expr::validate`, `Expr::sample`.
- Internal helper in sampling: `with_clip_local` remaps both `clip_local` and corresponding global frame.

### 4.6 `wavyte-core/src/animation/ops.rs`
Purpose:
- Ergonomic constructors around `Anim::Expr`.

Functions:
- `delay`, `speed`, `reverse`, `loop_`, `mix`.
- `sequence(a, a_len, b)` builds step-like switch with delayed `b`.
- `stagger(Vec<(offset, Anim<f64>)>)` sorts by offsets and chains via `sequence`.

### 4.6.1 `wavyte-core/src/transform/*`
Purpose:
- Host transform-domain helper modules used for future separation of linear/affine/non-linear concerns.

Files:
- `wavyte-core/src/transform/linear.rs`: vector interpolation helpers (`lerp_vec2`).
- `wavyte-core/src/transform/affine.rs`: affine composition/identity helpers.
- `wavyte-core/src/transform/non_linear.rs`: non-linear utility helpers (`clamp01`).

### 4.7 `wavyte-core/src/composition/model.rs`
Purpose:
- Canonical timeline data model + validation rules.

Types:
- `Composition { fps, canvas, duration, assets, tracks, seed }`.
- `Track` with z-base and layout controls:
  - `layout_mode`, `layout_gap_px`, `layout_padding`, `layout_align_x`, `layout_align_y`, `layout_grid_columns`.
- Layout enums/structs:
  - `LayoutMode`: `Absolute | HStack | VStack | Grid | Center`
  - `Edges`
  - `LayoutAlignX`, `LayoutAlignY`
- `Clip { id, asset, range, props, z_offset, effects, transition_in, transition_out }`.
- `ClipProps { transform, opacity, blend }`.
- `BlendMode::Normal`.
- `Asset` variants:
  - `Text`, `Svg`, `Path`, `Image`, `Video`, `Audio`.
- Media asset structs:
  - `VideoAsset` and `AudioAsset` with trim/rate/volume/fades/mute controls.
- `EffectInstance { kind, params }`.
- `TransitionSpec { kind, duration_frames, ease, params }`.

Validation flow:
- `Composition::validate` enforces:
  - fps/canvas/duration non-zero.
  - layout fields finite and valid.
  - grid columns > 0 for grid mode.
  - clip asset references exist.
  - clip ranges are valid and within composition duration.
  - anim props validate.
  - transition specs validate.
  - asset keys non-empty.
  - source paths relative and no `..`.
  - text constraints (non-empty text, finite positive size/width).
  - media controls finite and legal.
  - path `svg_path_d` non-empty.
- Helpers:
  - `validate_rel_source`.
  - `validate_media_controls`.
- `TransitionSpec::validate` ensures kind non-empty, positive duration, params null/object.

### 4.8 `wavyte-core/src/composition/dsl.rs`
Purpose:
- Programmatic builder DSL over model types.

Types/builders:
- `CompositionBuilder`.
- `TrackBuilder`.
- `ClipBuilder`.

Functions/impl:
- `CompositionBuilder`:
  - `new`, `seed`, `asset`, `track`, `video_asset`, `audio_asset`, `build`.
- Free helpers:
  - `video_asset(source) -> VideoAsset` default controls.
  - `audio_asset(source) -> AudioAsset` default controls.
- `TrackBuilder`:
  - `new`, `z_base`, `clip`, `layout_mode`, `layout_gap_px`, `layout_padding`, `layout_align`, `layout_grid_columns`, `build`.
- `ClipBuilder`:
  - `new`, `z_offset`, `opacity`, `transform`, `effect`, `transition_in`, `transition_out`, `build`.

### 4.9 `wavyte-core/src/layout/solver.rs`
Purpose:
- Resolve track-level layout offsets from intrinsic asset sizes.

Types:
- `LayoutOffsets { per_track: Vec<Vec<Vec2>> }`.

Functions:
- `LayoutOffsets::offset_for(track_idx, clip_idx)` with zero fallback.
- `resolve_layout_offsets(comp, assets)`.
- Internal:
  - `resolve_track_offsets` implements layout modes.
  - `intrinsic_size_for_asset_key` from prepared assets:
    - image/svg/video dimensions
    - text line metrics aggregation
    - path bbox
    - audio -> `(0,0)`
  - `align_offset` generic start/center/end logic.
  - `AlignKind` and `From<LayoutAlignX/Y>` conversions.

### 4.10 `wavyte-core/src/eval/evaluator.rs`
Purpose:
- Per-frame evaluation from timeline to visible render nodes.

Types:
- `EvaluatedGraph { frame, nodes }`.
- `EvaluatedClipNode`:
  - clip/asset ids, z, affine transform, clamped opacity, blend, optional video source time, resolved effects/transitions.
- `ResolvedEffect { kind, params }`.
- `ResolvedTransition { kind, progress, params }`.
- `Evaluator` unit struct.

Functions:
- `Evaluator::eval_frame` delegates to layout-aware evaluation with default offsets.
- `Evaluator::eval_frame_with_layout`:
  - validates composition.
  - delegates to shared implementation (`eval_frame_with_layout_impl`).
- `Evaluator::eval_frame_with_layout_unchecked` (internal):
  - skips composition validation for hot loops after API-boundary validation.
- Shared implementation:
  - bounds-check frame.
  - collects visible clips.
  - computes node sort key `(z, track_index, clip_start, clip_id)`.
- Internal:
  - `eval_clip` samples `opacity` and `transform` anims, merges layout offset, computes deterministic clip seed, resolves source time for videos.
  - `resolve_effect` (basic structural check).
  - `resolve_transition_in`, `resolve_transition_out`.
  - `resolve_transition_window` (computes transition progress on edge window).
  - `stable_hash64(seed, clip_id)`.

### 4.11 `wavyte-core/src/assets/decode.rs`
Purpose:
- Raw bytes -> prepared image/svg helper decode.

Functions:
- `decode_image(bytes) -> PreparedImage`
  - decodes via `image`, converts to RGBA8, premultiplies in-place.
- `parse_svg(bytes) -> PreparedSvg`
  - parses via `usvg::Tree::from_data` default options.
- Internal: `premultiply_rgba8_in_place`.

### 4.12 `wavyte-core/src/assets/svg_raster.rs`
Purpose:
- Convert SVG vectors to raster pixmaps with scaling-aware sizing.

Types:
- `SvgRasterKey { asset, width, height }` used by CPU cache.

Functions:
- `svg_raster_params(tree, transform) -> (width, height, transform_adjust)`:
  - infers scale from affine coefficients.
  - computes conservative raster dimensions.
  - caps dimensions at 16384.
  - returns adjusted transform to draw rasterized result in original scene space.
- `rasterize_svg_to_premul_rgba8(tree, width, height)` via `resvg`/`tiny_skia`.

### 4.13 `wavyte-core/src/assets/media.rs`
Purpose:
- Media probing/decoding and timeline->source time mapping.

Types:
- `VideoSourceInfo { source_path, width, height, fps_num, fps_den, duration_sec, has_audio }`.
- `AudioPcm { sample_rate, channels, interleaved_f32 }`.
- Constant: `MIX_SAMPLE_RATE = 48000`.

Functions:
- `VideoSourceInfo::source_fps`.
- `video_source_time_sec(video_asset, clip_local_frames, fps)`.
- `audio_source_time_sec(audio_asset, clip_local_frames, fps)`.
- Feature-gated decode/probe (`media-ffmpeg`):
  - `probe_video` using `ffprobe` JSON.
  - `decode_video_frame_rgba8` via `ffmpeg -f rawvideo -pix_fmt rgba`.
  - `decode_video_frames_rgba8` batched decode helper (crate-internal) used by CPU prefetch.
  - `decode_audio_f32_stereo` via `ffmpeg` f32le decode; gracefully returns empty PCM for no-audio streams.
- Non-feature stubs return evaluation errors.
- Internal helper under feature: `parse_ff_ratio`.

### 4.14 `wavyte-core/src/assets/store.rs`
Purpose:
- Immutable prepared asset store with deterministic ID/keying.

Visual (prepare path):

```text
Composition.assets (model keys)
    |
    | for each asset key:
    v
 key_for(asset) -> (kind_tag, AssetKey{norm_path,params})
    |
    +--> hash_id_for_key(kind_tag, AssetKey) -> AssetId
    |
    +--> prepare payload
          Image: read bytes -> decode_image -> PreparedImage
          Svg  : read bytes -> parse_svg_with_options -> PreparedSvg
          Text : read font -> TextLayoutEngine::layout_plain -> PreparedText
          Path : parse_svg_path -> PreparedPath
          Video: probe_video (+ optional decode_audio) -> PreparedVideo
          Audio: decode_audio_f32_stereo -> PreparedAudio
    |
    v
 ids_by_key[model_key] = AssetId
 assets_by_id[AssetId] = PreparedAsset
```

Prepared asset types:
- `PreparedImage`.
- `PreparedSvg`.
- `PreparedText` (Parley layout + raw font bytes + family name).
- `PreparedPath`.
- `PreparedAudio`.
- `PreparedVideo` (video info + optional decoded audio).
- `PreparedAsset` enum over all prepared kinds.

Identity/keying:
- `AssetId(u64)` with `from_u64`, `as_u64`.
- `AssetKey { norm_path, params }` with sorted params.
- Shared `foundation/math.rs::Fnv1a64` hasher for stable IDs.

Store:
- `PreparedAssetStore { root, ids_by_key, assets_by_id }`.
- `PreparedAssetStore::prepare(comp, root)`:
  - iterates model assets.
  - computes `(kind_tag, AssetKey)`.
  - hashes deterministic `AssetId`.
  - prepares each asset:
    - image/svg/text/path/video/audio.
- Lookup APIs:
  - `root()`
  - `id_for_key(model_asset_key)`
  - `get(asset_id)`

Internals:
- `key_for(asset)` builds canonical keys (text keys include content + style bits).
- `hash_id_for_key`.
- `read_bytes`.
- `parse_svg_with_options` with font/resource options.
- SVG fontdb helpers:
  - `build_svg_fontdb`, `load_fonts_from_dir`, `make_svg_font_resolver`.
- `normalize_rel_path` (slash normalization, relative-only, no `..`).
- `parse_svg_path` for inline path assets.

Text layout engine:
- `TextBrushRgba8` brush payload.
- `TextLayoutEngine`:
  - `new/default`.
  - `last_family_name`.
  - `layout_plain(text, font_bytes, size, brush, max_width)` via Parley.

### 4.15 `wavyte-core/src/effects/transitions.rs`
Purpose:
- Parse transition kinds and params into typed runtime variants.

Types:
- `WipeDir` (`LeftToRight`, `RightToLeft`, `TopToBottom`, `BottomToTop`).
- `TransitionKind` (`Crossfade`, `Wipe { dir, soft_edge }`).

Functions:
- `parse_transition_kind_params(kind, params)`.
- `parse_transition(&TransitionSpec)` convenience.

Details:
- Accepts aliases for wipe direction (`ltr`, `rtl`, `ttb`, `btt`, etc).
- `soft_edge` is clamped `[0,1]` and must be finite.

### 4.16 `wavyte-core/src/effects/fx.rs`
Purpose:
- Parse effect instances and normalize into inline vs pass pipeline.

Types:
- `Effect`:
  - `OpacityMul { value }`
  - `TransformPost { value: Affine }`
  - `Blur { radius_px, sigma }`
- `InlineFx { opacity_mul, transform_post }`.
- `PassFx::Blur`.
- `FxPipeline { inline, passes }`.

Functions:
- `parse_effect` with accepted aliases:
  - opacity_mul variants
  - transform_post variants
  - blur
- `normalize_effects`:
  - folds opacity multipliers multiplicatively.
  - composes transform posts.
  - emits blur pass entries for non-zero radius.
  - returns default/noop pipeline when empty identity.
- Internal param parsing helpers:
  - `get_u32`, `get_f32`, `parse_affine`.

### 4.17 `wavyte-core/src/compile/fingerprint.rs`
Purpose:
- Hash evaluated frame content for static-frame elision.

Types:
- `FrameFingerprint { hi, lo }` dual-hash tuple.
- Shared `foundation/math.rs::Fnv1a64` hasher.

Functions:
- `fingerprint_eval(eval)` hashes all semantically relevant node fields:
  - identity, z, transform coeffs, opacity, blend, source time, effects, transitions, params.
- JSON canonicalization helper:
  - `write_json_value_pair` sorts object keys before hashing.
- Pair-write helpers:
  - `write_u8_pair`, `write_u64_pair`, `write_i64_pair`, `write_str_pair`.

### 4.18 `wavyte-core/src/compile/plan.rs`
Purpose:
- Compile evaluated graph into backend-agnostic render IR.

Visual (layer compile and transition pairing):

```text
Evaluated nodes (already z-sorted)
   node0, node1, node2, ...
      |
      +-- each node -> ScenePass(target=surface_i, ops=[DrawOp])
      +-- optional pass fx -> OffscreenPass chain (surface_i -> surface_j -> ...)
      +-- push final layer surface id into layer list

Layer list: [L0, L1, L2, ...] where each Li has:
  - rendered surface id
  - optional transition_in/out

Composite build scan:
  for i in 0..layers.len():
    try pair (Li.out, L{i+1}.in) if:
      - transition kind compatible
      - progress aligned (abs diff <= 0.05)
      - wipe params compatible if wipe
    if paired:
      emit CompositeOp::Crossfade/Wipe
      i += 2
    else:
      emit CompositeOp::Over with transition attenuated opacity
      i += 1

Final pass order:
  [all scene/offscreen passes ...] + [one CompositePass(target=Surface0)]
```

IR types:
- `RenderPlan { canvas, surfaces, passes, final_surface }`.
- `Pass`: `Scene`, `Offscreen`, `Composite`.
- `ScenePass { target, ops, clear_to_transparent }`.
- `SurfaceId(u32)`.
- `PixelFormat::Rgba8Premul`.
- `SurfaceDesc { width, height, format }`.
- `OffscreenPass { input, output, fx }`.
- `CompositePass { target, ops }`.
- `CompositeOp`:
  - `Over { src, opacity }`
  - `Crossfade { a, b, t }`
  - `Wipe { a, b, t, dir, soft_edge }`
- `DrawOp`:
  - `FillPath`, `Image`, `Svg`, `Text`, `Video`.

Functions:
- `compile_frame(comp, eval, assets)`:
  - convenience wrapper that creates a fresh `CompileCache`.
- `compile_frame_with_cache(comp, eval, assets, cache)` (crate-internal hot path):
  - starts with final surface 0.
  - for each evaluated node:
    - parse effects through cache (`parse_effect_cached`) using hashed keys plus equality-checked buckets (avoids per-lookup JSON string serialization).
    - combine clip opacity with inline opacity effect.
    - apply inline transform post.
    - map asset type -> draw op.
    - scene pass renders node into its own layer surface.
    - pass effects append offscreen passes chaining surfaces.
  - transition composition stage:
    - parses transitions through the same hashed/equality-checked cache strategy (`parse_transition_cached`).
    - tries pairing adjacent layers when outgoing/incoming transition kinds align and progress is near-equal.
    - emits single `Crossfade` or `Wipe` op for paired transitions.
    - otherwise emits `Over` with in/out attenuation.
  - appends final composite pass targeting surface 0.

### 4.19 `wavyte-core/src/effects/composite.rs`
Purpose:
- Premultiplied-alpha CPU compositing primitives.

Types:
- `type PremulRgba8 = [u8; 4]`.
- `WipeParams { width, height, t, dir, soft_edge }`.

Functions:
- Pixel ops:
  - `over(dst, src, opacity)`
  - `crossfade(a, b, t)`
- Buffer ops:
  - `over_in_place(dst, src, opacity)`
  - `crossfade_over_in_place(dst, a, b, t)`
  - `wipe_over_in_place(dst, a, b, params)`
- Helpers:
  - `mul_div255` (delegates to `foundation/math.rs`), `add_sat_u8`, `smoothstep`.

### 4.20 `wavyte-core/src/effects/blur.rs`
Purpose:
- CPU separable Gaussian blur on premultiplied RGBA8 buffers.

Functions:
- `blur_rgba8_premul(src, width, height, radius, sigma)`.
- Internal:
  - `gaussian_kernel_q16` (normalized fixed-point kernel).
  - `horizontal_pass`, `vertical_pass`.
  - `q16_to_u8`.

### 4.21 `wavyte-core/src/render/passes.rs`
Purpose:
- Generic pass execution contract shared by backends.

Types/traits:
- `PassBackend` trait with hooks:
  - `ensure_surface`
  - `exec_scene`
  - `exec_offscreen`
  - `exec_composite`
  - `readback_rgba8`

Functions:
- `execute_plan(backend, plan, assets)`:
  - creates all surfaces.
  - executes pass list in order.
  - reads back final surface into `FrameRGBA`.

### 4.22 `wavyte-core/src/render/backend.rs`
Purpose:
- Backend abstraction and factory.

Types:
- `FrameRGBA { width, height, data, premultiplied }`.
- `RenderBackend` trait (extends `PassBackend`):
  - default `render_plan` delegates to `execute_plan`.
  - optional `worker_render_settings` for parallel workers.
- `BackendKind::Cpu`.
- `RenderSettings { clear_rgba }`.

Functions:
- `create_backend(kind, settings)` -> boxed backend (`CpuBackend`).

### 4.23 `wavyte-core/src/render/cpu.rs`
Purpose:
- CPU backend implementation using `vello_cpu` plus project-specific raster/composite logic.

Visual (pass execution over surfaces):

```text
ensure_surface(id, desc)
  -> allocate or reuse CpuSurface pixmap (resize only when dimensions change)
  -> optional clear on final surface 0

ScenePass(target=T, ops=[...])
  -> remove surface T from map
  -> record draw ops in vello_cpu RenderContext
  -> flush + render_to_pixmap
  -> insert surface T back

OffscreenPass(input=A, output=B, fx=Blur)
  -> copy/read input bytes
  -> blur_rgba8_premul(...)
  -> write output bytes

CompositePass(target=0, ops=[...])
  -> for each op:
       Over      -> over_in_place(dst, src, opacity)
       Crossfade -> crossfade_over_in_place(dst, a, b, t)
       Wipe      -> wipe_over_in_place(dst, a, b, params)

readback_rgba8(final_surface)
  -> retain only surfaces declared in current plan (drop stale extras)
  -> FrameRGBA { premultiplied=true }
```

Core types:
- `CpuBackend` caches and surfaces:
  - `image_cache`, `svg_cache`, `font_cache`, `video_decoders`, `surfaces`.
- `CpuSurface { width, height, pixmap }`.
- `VideoFrameDecoder`:
  - frame cache + LRU by rounded milliseconds.
  - `capacity` from env `WAVYTE_VIDEO_CACHE_CAPACITY` (default `64`).
  - `prefetch_frames` from env `WAVYTE_VIDEO_PREFETCH_FRAMES` (default `12`).
  - prefetch path uses batched `decode_video_frames_rgba8`.

PassBackend impl:
- `ensure_surface`:
  - allocates/reset surfaces.
  - clears final surface from `RenderSettings` if configured.
- `exec_scene`:
  - optional transparent clear.
  - records draw ops in `vello_cpu::RenderContext`.
- `exec_offscreen`:
  - currently handles blur pass.
  - supports in-place and separate input/output cases.
- `exec_composite`:
  - applies `Over`, `Crossfade`, `Wipe` by calling `effects/composite.rs`.
- `readback_rgba8`:
  - converts target pixmap to `FrameRGBA`.
  - trims stale surfaces not used by the just-executed plan.

RenderBackend impl:
- `worker_render_settings` returns cloneable settings (required for parallel workers).

Draw path internals:
- `draw_op` dispatches `DrawOp` variants:
  - path fill
  - image draw
  - text glyph runs via Parley layout and font cache
  - svg draw via cached rasterization
  - video draw via frame decoder
- Geometry conversion helpers:
  - `affine_to_cpu`, `point_to_cpu`, `bezpath_to_cpu`.
- Pixel conversion helpers:
  - `premul_rgba8`, `clear_pixmap`, `image_premul_bytes_to_pixmap`, `image_paint_size`.
- Asset-specific cache loaders:
  - `image_paint_for`, `font_for_text_asset`, `svg_paint_for`, `video_paint_for`.

### 4.24 `wavyte-core/src/audio/mix.rs`
Purpose:
- Build an audio segment manifest from timeline clips and mix to stereo float PCM.

Visual (audio segment mapping):

```text
Timeline frame range [R.start, R.end)
   |
   +-- for each clip intersecting range:
         if AudioAsset: use PreparedAudio
         if VideoAsset: use PreparedVideo.audio when present
         else: ignore
   |
   +-- convert frame deltas to timeline sample indices
   +-- compute source_start_sec from trim/rate and clip-local frame
   +-- keep fades/volume/playback_rate metadata
   v
AudioManifest { total_samples, segments[] }
   |
   +-- mix_manifest:
         for each segment sample:
           src_sec = source_start_sec + rel_sec * playback_rate
           apply fade in/out gain * volume
           add to output L/R
         clamp [-1,1]
```

Types:
- `AudioSegment` describes one clip contribution in timeline sample space.
- `AudioManifest { sample_rate, channels, total_samples, segments }`.

Functions:
- `build_audio_manifest(comp, assets, range)`:
  - intersects clip ranges with requested range.
  - extracts audio from `AudioAsset` and `VideoAsset` (if prepared video has audio).
  - computes source/timeline mapping and fades.
- `mix_manifest(manifest)`:
  - maps timeline sample -> source position via playback rate.
  - performs linear interpolation between adjacent source samples.
  - applies fades and volume.
  - sums overlapping segments and clamps to `[-1, 1]`.
- `write_mix_to_f32le_file(samples, out_path)`.
- Helpers:
  - `fade_gain`.
  - `push_audio_segment`, `push_video_audio_segment`, `push_segment_common`.
  - `frame_to_sample` (rounded rational conversion).
  - `intersect_ranges`.

### 4.25 `wavyte-core/src/encode/ffmpeg.rs`
Purpose:
- Streaming MP4 encoder wrapper over system `ffmpeg` process.

Visual (frame flatten + ffmpeg streaming):

```text
FrameRGBA (possibly premul alpha)
   |
   +-- flatten_to_opaque_rgba8(dst, src, src_is_premul, bg_rgba)
   |      if premul:   rgb_out = src_rgb + bg_rgb*(1-a)
   |      if straight: rgb_out = src_rgb*a + bg_rgb*(1-a)
   |      alpha_out = 255
   |
   +-- write dst bytes to ffmpeg stdin (rawvideo rgba)
   |
   +-- ffmpeg transcode -> libx264 yuv420p mp4 (+ optional AAC)
```

Types:
- `EncodeConfig { width, height, fps, out_path, overwrite, audio }`.
- `AudioInputConfig { path, sample_rate, channels }`.
- `FfmpegEncoder { cfg, bg_rgba, child, stdin, stderr_drain, scratch }`.

Functions/impl:
- `EncodeConfig::validate`:
  - non-zero size/fps.
  - even dimensions (yuv420p).
  - valid audio settings when enabled.
- `EncodeConfig::with_out_path`.
- `default_mp4_config`.
- `is_ffmpeg_on_path`.
- `ensure_parent_dir`.
- `FfmpegEncoder::new`:
  - validates config, checks ffmpeg availability, builds command line.
  - supports optional f32le audio input + AAC output.
  - starts background stderr drain thread to avoid blocked stderr pipe.
- `encode_frame`:
  - dimension checks.
  - flattens alpha into opaque RGBA scratch buffer.
  - writes to ffmpeg stdin.
- `finish` closes stdin, waits for process, joins stderr drain, and validates status.
- Internal flattening helpers:
  - `flatten_to_opaque_rgba8`, `mul_div255`.

### 4.26 `wavyte-core/src/render/pipeline.rs`
Purpose:
- High-level render orchestration APIs (single frame, batch, MP4) including parallel/chunk modes.

Visual (parallel chunk worker model):

```text
render_frames_with_stats(parallel=true)
  |
  +-- build rayon thread pool (optional fixed thread count)
  +-- split requested range into chunk windows
  +-- per chunk:
       eval all frames sequentially in caller thread
       if static_frame_elision:
         fingerprint each evaluated frame
         dedupe by fingerprint
       else:
         treat all evaluated frames as unique (no fingerprint pass)
       rayon workers:
         for each unique eval
           local CpuBackend(settings clone)
           compile_frame_with_cache(worker-local cache) + render_plan
       reconstruct original order
       accumulate RenderStats
```

Public types:
- `RenderThreading { parallel, chunk_size, threads, static_frame_elision }`.
- `RenderStats { frames_total, frames_rendered, frames_elided }`.
- `RenderToMp4Opts { range, bg_rgba, overwrite, threading }`.

Public functions:
- `render_frame`:
  - validates composition once.
  - resolve layout -> eval (unchecked) -> compile (cache-backed) -> execute.
- `render_frames` wrapper over `render_frames_with_stats`.
- `render_frames_with_stats`:
  - validates composition once.
  - sequential or parallel chunk flow.
  - in sequential mode uses shared backend + rolling `CompileCache`.
  - parallel mode requires backend worker settings and threadpool.
- `render_to_mp4` wrapper over `render_to_mp4_with_stats`.
- `render_to_mp4_with_stats`:
  - validates composition once, then validates range/fps.
  - checks ffmpeg availability.
  - builds/mixes temporary audio input if needed.
  - chunk-renders frames and streams them into encoder.
  - parallel MP4 path keeps unique frames + `frame_to_unique` mapping and encodes without cloning duplicate frame buffers.

Internal functions:
- `render_chunk_sequential`.
- `render_chunk_parallel_cpu_unique`:
  - evaluates all frames in chunk.
  - computes fingerprints only when `static_frame_elision` is enabled.
  - when elision is disabled, skips fingerprinting and treats all frames as unique.
  - renders unique frames in parallel via rayon `map_init` worker backends.
  - returns unique frame vector + frame-to-unique mapping.
- `render_chunk_parallel_cpu`:
  - adapter used by `render_frames_with_stats` that reconstructs ordered output by clone/move reuse.
- `build_thread_pool`.
- `normalized_chunk_size`.
- `TempFileGuard` deletes temp audio file on drop.

### 4.27 `wavyte-cli/src/main.rs`
Purpose:
- CLI entrypoint for JSON composition rendering.

CLI model:
- `Cli` root with subcommands.
- `Command`: `Frame` and `Render`.
- `FrameArgs` and `RenderArgs`.
- `BackendChoice` currently `Cpu`.

Functions:
- `main` parse and dispatch.
- `read_comp_json` from disk.
- `make_backend` selects backend.
- `cmd_frame`:
  - validate composition.
  - prepare assets relative to input file directory.
  - optional font/SVG diagnostics.
  - render single frame and write PNG.
- `cmd_render`:
  - validate/prepare.
  - render full duration to MP4 with default threading.
- Diagnostics helpers:
  - `dump_font_diagnostics`.
  - `sha256_hex`.
  - `count_svg_text_nodes`.

### 4.28 `wavyte-core/src/guide.rs`
Purpose:
- Narrative rustdoc module introducing architecture and usage.

Content:
- Defines key concepts and staged pipeline.
- Documents no-IO renderer rule and premultiplied alpha contract.
- Includes minimal no-run example using DSL + CPU backend.

### 4.29 `wavyte-core/src/lib.rs`
Purpose:
- Crate root, module wiring, and public re-export surface.

Responsibilities:
- Declares `#![forbid(unsafe_code)]`.
- Maps foldered files to stable module names using `#[path = ...]`.
- Re-exports all public API surface from foundation/animation/composition/assets/render/audio/encoding.
- Exposes `pub mod guide`.

## 5. Runtime behavior notes that matter for perf and correctness

1. Track layout is resolved once per API call and injected into eval.
- `render_frame`, `render_frames_with_stats`, and `render_to_mp4_with_stats` all resolve layout offsets before evaluation loops/chunks.
- These APIs also validate composition once up front, then run unchecked eval in frame hot loops.

2. Parallel rendering parallelizes eval+compile+render for unique frames, not encoding.
- Encoding still runs in caller thread, so encode cost can dominate at higher resolutions.

3. Static-frame elision is fingerprint-based.
- Only exact evaluated-scene matches are deduped.
- Output ordering remains stable even when deduping.
- When `static_frame_elision` is disabled, the parallel path skips fingerprint computation.

4. SVG rendering path is raster cache keyed by `(asset_id, raster_w, raster_h)`.
- Transform scale affects raster size selection.
- Prevents repeated costly vector rasterization for identical scale contexts.

5. Video frame caching in CPU backend is tunable and prefetches in batches.
- Keys are rounded milliseconds.
- `WAVYTE_VIDEO_CACHE_CAPACITY` and `WAVYTE_VIDEO_PREFETCH_FRAMES` control cache/prefetch windows.
- Batched decode reduces ffmpeg process churn for temporally local access.

6. Audio path is separate from visual rendering.
- Audio manifest/mix happens before ffmpeg spawn path in MP4 rendering.
- Mixed temporary file is removed via guard.

## 6. Known source-level wording drift

There are a few source comments/error strings that still say `v0.1` while living in v0.2.1 code. They do not change behavior but can confuse readers. Notable locations include:
- `animation/proc.rs` unsupported type error text.
- `effects/fx.rs` blur radius text.
- some doc comments in render modules.

## 7. Non-src code surfaces (runtime context)

This doc is `wavyte-core/src/`-first, but these files define how the crate is exercised in practice:

- `bench/src/main.rs`
  - End-to-end benchmark harness with warmup/repeat controls.
  - Measures stage timings: backend create, ffmpeg spawn/finish, eval/compile/render, encode write, wall.
  - Supports sequential and parallel modes, thread count, chunking, optional static-frame elision.
  - Builds a representative composition using path/svg/image/text (and transitions/effects optionally).

- `wavyte-core/examples/*.rs`
  - Demonstrate single-frame rendering, PNG output, MP4 output, transition/effect examples, layout examples, media-gamut example, and Remotion-style hello-world composition.
  - Most MP4 examples route through `wavyte-core/examples/support/mod.rs` helper wrappers.
  - Examples intentionally emit artifacts into repo `assets/` for quick visual validation.

- `wavyte-core/tests/*.rs`
  - Integration tests cover asset preparation, JSON/validation, evaluator behavior, CPU renderer, SVG/text paths, parallel parity, and media pipeline paths (feature-gated).
- `wavyte-cli/tests/*.rs`
  - CLI integration tests (including frame smoke flow invoking binary `wavyte`).
  - Unit tests inside modules assert low-level invariants (math, parse, transitions, blur/composite correctness, etc).

## 8. Public API call-chain atlas (final pass)

Scope of this section:
- Primary target: callable APIs exported through `wavyte-core/src/lib.rs` (functions + public methods on exported types).
- Secondary addendum: selected `pub` helpers inside private modules that are public in source but not re-exported.

Legend:
- `->` means direct runtime call/flow.
- `=>` means pure transform/check (no downstream module call).
- `[error]` marks explicit error exits.
- `(...)` means important arguments omitted for brevity.

### 8.1 Foundation APIs

#### 8.1.1 `WavyteError` constructors

```text
WavyteError::validation(msg)
  => msg.into()
  => WavyteError::Validation

WavyteError::animation(msg)
  => msg.into()
  => WavyteError::Animation

WavyteError::evaluation(msg)
  => msg.into()
  => WavyteError::Evaluation

WavyteError::serde(msg)
  => msg.into()
  => WavyteError::Serde
```

#### 8.1.2 `FrameRange` methods

```text
FrameRange::new(start,end)
  => check start <= end
  -> Ok(FrameRange) | [error Validation]

FrameRange::len_frames()
  => end.saturating_sub(start)

FrameRange::is_empty()
  => start == end

FrameRange::contains(f)
  => start <= f < end

FrameRange::clamp(f)
  => if empty: start
  => else clamp to [start, end-1]

FrameRange::shift(delta)
  => shift start and end with saturating add/sub
  => new FrameRange
```

#### 8.1.3 `Fps` methods

```text
Fps::new(num,den)
  => check den>0 and num>0
  -> Ok(Fps) | [error Validation]

Fps::as_f64()
  => num / den

Fps::frame_duration_secs()
  => den / num

Fps::frames_to_secs(frames)
  -> frame_duration_secs()
  => frames * frame_duration_secs

Fps::secs_to_frames_floor(secs)
  -> as_f64()
  => floor(secs * fps)
```

#### 8.1.4 `Rgba8Premul` and `Transform2D`

```text
Rgba8Premul::transparent()
  => {0,0,0,0}

Rgba8Premul::from_straight_rgba(r,g,b,a)
  => per channel premul ((c*a)+127)/255
  => Rgba8Premul

Transform2D::to_affine()
  => T(translate) * T(anchor) * R(rotation) * S(scale) * T(-anchor)
  => kurbo::Affine
```

### 8.2 Animation APIs

#### 8.2.1 Easing

```text
Ease::apply(t)
  => clamp t to [0,1]
  => evaluate selected curve polynomial
```

#### 8.2.2 `Anim<T>` and `Keyframes<T>`

```text
Anim::constant(value)
  => build Keyframes with one key at frame 0 (Hold, Linear ease metadata)

Anim::validate()
  -> Keyframes::validate() | Procedural OK | Expr::validate()

Anim::sample(ctx)
  -> Keyframes::sample(ctx) | Procedural::sample(ctx) | Expr::sample(ctx)

Keyframes::validate()
  => require keys non-empty OR default set
  => require keys sorted by frame
  -> Ok | [error Animation]

Keyframes::sample(ctx)
  => if no keys: default or [error Animation]
  => binary-partition key index by clip_local frame
  => edge cases before first / after last
  => interpolate or hold between adjacent keys
  -> sampled value
```

#### 8.2.3 Expression helpers (`delay/speed/reverse/loop_/mix/sequence/stagger`)

```text
delay(inner,by)   => Anim::Expr(Delay{...})
speed(inner,f)    => Anim::Expr(Speed{...})
reverse(inner,d)  => Anim::Expr(Reverse{...})
loop_(inner,p,m)  => Anim::Expr(Loop{...})
mix(a,b,t)        => Anim::Expr(Mix{...})

sequence(a,a_len,b)
  -> delay(b,a_len)
  -> build step Anim<f64> (0 before boundary, 1 at/after boundary)
  -> mix(a,b_delayed,step)

stagger([(offset,anim)...])
  => sort by offset
  => fold with sequence(...)
  -> composite Anim<f64>
```

#### 8.2.4 `Expr<T>` runtime

```text
Expr::validate()
  => check factor/period/duration invariants when relevant
  -> recursively validate inner anim(s)

Expr::sample(ctx)
  => remap clip_local/frame according to variant:
     Delay    : max(clip_local-by,0)
     Speed    : floor(clip_local*factor)
     Reverse  : (duration-1)-min(clip_local,duration-1)
     Loop     : repeat/pingpong mapping
     Mix      : sample t,a,b then lerp(a,b,clamp(t))
  -> sample underlying anim(s)
```

### 8.3 Composition model + DSL APIs

#### 8.3.1 Model validation

```text
Composition::validate()
  => validate fps/canvas/duration basics
  => validate track layout fields
  => validate clip references and ranges
  -> clip.props.opacity.validate()
  -> clip.props.transform.validate()
  -> transition.validate() for in/out when present
  => validate each asset payload/path/media controls
  -> Ok | [error Validation]

TransitionSpec::validate()
  => kind non-empty
  => duration_frames > 0
  => params null or object
```

#### 8.3.2 `CompositionBuilder`

```text
CompositionBuilder::new(fps,canvas,duration)
  => initialize empty builder state

seed(seed) => set seed
asset(key,asset)
  => reject duplicate key
  => insert into map
track(track) => push track

video_asset(key,source)
  -> dsl::video_asset(source)
  -> asset(key, Asset::Video(...))

audio_asset(key,source)
  -> dsl::audio_asset(source)
  -> asset(key, Asset::Audio(...))

build()
  => assemble Composition
  -> Composition::validate()
  -> Ok(Composition) | [error]
```

#### 8.3.3 `TrackBuilder`

```text
TrackBuilder::new(name) => defaults (z=0, Absolute layout, grid cols=2)
z_base(z)               => set z base
clip(clip)              => push clip
layout_mode(mode)       => set mode
layout_gap_px(gap)      => set gap
layout_padding(edges)   => set padding
layout_align(x,y)       => set align
layout_grid_columns(c)  => set columns
build()
  => name non-empty check
  -> Ok(Track) | [error Validation]
```

#### 8.3.4 `ClipBuilder`

```text
ClipBuilder::new(id,asset_key,range) => defaults (opacity=1, identity transform)
z_offset(z)            => set z offset
opacity(anim)          => set opacity anim
transform(anim)        => set transform anim
effect(fx)             => push effect
transition_in(tr)      => set transition_in
transition_out(tr)     => set transition_out
build()
  => id and asset key non-empty checks
  -> opacity.validate()
  -> transform.validate()
  -> Ok(Clip) | [error]
```

#### 8.3.5 DSL free helpers

```text
video_asset(source)
  => construct VideoAsset with default trim/rate/volume/fade/mute

audio_asset(source)
  => construct AudioAsset with default trim/rate/volume/fade/mute
```

### 8.4 Asset/media/store APIs

#### 8.4.1 Store identity helpers

```text
AssetId::from_u64(raw) => AssetId(raw)
AssetId::as_u64()      => raw u64

AssetKey::new(norm_path, params)
  => sort params
  => AssetKey

normalize_rel_path(source)
  => normalize '\\' to '/'
  => reject absolute path, '..', empty result
  => join normalized parts
```

#### 8.4.2 `PreparedAssetStore`

```text
PreparedAssetStore::prepare(comp, root)
  => initialize store maps + TextLayoutEngine
  for each model asset:
    -> key_for(asset)                            # normalized key + params
    -> hash_id_for_key(kind_tag, AssetKey)      # deterministic AssetId
    -> prepare payload by kind:
         Image: read_bytes -> decode_image
         Svg  : read_bytes -> parse_svg_with_options
         Text : read_bytes(font) -> TextLayoutEngine::layout_plain
         Path : parse_svg_path
         Video: probe_video (+ decode_audio_f32_stereo when has_audio)
         Audio: decode_audio_f32_stereo
    => ids_by_key/model key and assets_by_id/AssetId entries
  -> Ok(store) | [error]

PreparedAssetStore::root()
  => return root path ref

PreparedAssetStore::id_for_key(model_key)
  => map lookup by model key
  -> AssetId | [error Evaluation unknown key]

PreparedAssetStore::get(asset_id)
  => map lookup by AssetId
  -> &PreparedAsset | [error Evaluation unknown id]
```

#### 8.4.3 Text layout engine

```text
TextLayoutEngine::new()
  => init Parley font context + layout context

TextLayoutEngine::last_family_name()
  => clone last resolved family string

TextLayoutEngine::layout_plain(text,font_bytes,size,brush,max_width)
  => validate size
  -> register font bytes in Parley collection
  => resolve family id and family name
  -> ranged_builder(...):
       default FontStack(family)
       default FontSize(size)
       default Brush(brush)
  -> build layout
  -> break lines (and align if max_width set)
  -> return layout
```

#### 8.4.4 Decode and media helpers

```text
decode_image(bytes)
  -> image::load_from_memory
  -> rgba8 conversion
  -> premultiply_rgba8_in_place
  -> PreparedImage

parse_svg(bytes)
  -> usvg::Tree::from_data(default options)
  -> PreparedSvg

VideoSourceInfo::source_fps()
  => fps_num/fps_den (or 0 if den=0)

video_source_time_sec(video_asset, clip_local_frames, fps)
  => timeline secs from frame and fps
  => trim_start + timeline*playback_rate
  => clamp to trim_end (if set) and >=0

audio_source_time_sec(audio_asset, clip_local_frames, fps)
  => same mapping as video_source_time_sec

probe_video(path) [feature media-ffmpeg]
  -> run ffprobe JSON
  => parse video stream and metadata
  -> VideoSourceInfo | [error]

decode_video_frame_rgba8(source, t) [feature media-ffmpeg]
  -> ffmpeg -ss t -frames:v 1 -f rawvideo -pix_fmt rgba pipe:1
  => length checks
  -> Vec<u8> RGBA | [error]

decode_video_frames_rgba8(source, start_t, frame_count) [feature media-ffmpeg, crate-internal]
  -> ffmpeg -ss start_t -frames:v N -f rawvideo -pix_fmt rgba pipe:1
  => validate batch byte length and slice into per-frame buffers
  -> Vec<Vec<u8>> | [error]

decode_audio_f32_stereo(path,sample_rate) [feature media-ffmpeg]
  -> ffmpeg decode to f32le stereo pipe
  => parse bytes to f32 samples
  -> AudioPcm | [error]
```

### 8.5 Evaluation/layout/compile/effect/transition APIs

#### 8.5.1 Layout

```text
resolve_layout_offsets(comp, assets)
  for each track:
    -> resolve_track_offsets(track):
         -> intrinsic_size_for_asset_key for each clip asset
         => compute offsets by layout mode:
            Absolute: all zero
            Center  : center each clip
            HStack  : horizontal flow + gap + align
            VStack  : vertical flow + gap + align
            Grid    : grid cells from max intrinsic cell size
  -> LayoutOffsets

LayoutOffsets::offset_for(track_idx, clip_idx)
  => nested vec lookup
  => default (0,0) if missing
```

#### 8.5.2 Evaluation

```text
Evaluator::eval_frame(comp, frame)
  -> eval_frame_with_layout(comp, frame, default LayoutOffsets)

Evaluator::eval_frame_with_layout(comp, frame, layout_offsets)
  -> eval_frame_with_layout_impl(..., validate_comp=true)

Evaluator::eval_frame_with_layout_unchecked(comp, frame, layout_offsets) [crate-internal]
  -> eval_frame_with_layout_impl(..., validate_comp=false)

eval_frame_with_layout_impl(...)
  => optional comp.validate()
  => frame bounds check
  for each track/clip visible at frame:
    -> eval_clip(...)
  => sort nodes by (z, track_index, clip_start, clip_id)
  -> EvaluatedGraph

eval_clip(...) [internal but central]
  => build SampleCtx with clip-local frame + stable clip seed
  -> opacity anim sample + clamp [0,1]
  -> transform anim sample + prepend layout translation
  -> source_time_s mapping for video assets
  -> resolve_effect list
  -> resolve transition_in/out windows
  -> EvaluatedClipNode
```

#### 8.5.3 Effects/transitions/fingerprinting

```text
parse_effect(inst)
  => parse kind aliases
  => validate params
  -> typed Effect | [error]

normalize_effects(effects)
  => fold opacity multipliers
  => compose transform_post affines
  => collect pass effects (blur radius>0)
  -> FxPipeline

parse_transition(spec)
  -> parse_transition_kind_params(spec.kind, spec.params)

fingerprint_eval(eval)
  => hash all semantically relevant node fields
  => canonicalize JSON object key ordering
  -> FrameFingerprint{hi,lo}
```

#### 8.5.4 Compilation

```text
compile_frame(comp, eval, assets)
  -> compile_frame_with_cache(comp, eval, assets, fresh cache)

compile_frame_with_cache(comp, eval, assets, cache) [crate-internal]
  => initialize final surface 0
  for each evaluated node:
    -> parse_effect_cached + normalize_effects
    => combine node opacity and inline opacity_mul
    => combine node transform and inline transform_post
    -> assets.id_for_key + assets.get
    => map prepared asset kind to DrawOp
    => allocate scene surface and ScenePass
    => append OffscreenPass chain for pass fx
    => push layer metadata (surface + transitions)
  => scan layers for transition pairing:
       transitions parsed via parse_transition_cached
       paired -> Crossfade/Wipe
       unpaired -> Over with transition attenuation
  => append final CompositePass(target=0)
  -> RenderPlan
```

### 8.6 Rendering backend + pass execution APIs

#### 8.6.1 Backend factory and construction

```text
create_backend(kind, settings)
  match kind:
    Cpu -> CpuBackend::new(settings.clone()) boxed as dyn RenderBackend

CpuBackend::new(settings)
  => initialize caches and surface map
```

#### 8.6.2 Trait-level call chain

```text
RenderBackend::render_plan(plan, assets) [default impl]
  -> execute_plan(self as PassBackend, plan, assets)

RenderBackend::worker_render_settings() [default]
  => None

CpuBackend::worker_render_settings() [override]
  => Some(settings clone)

execute_plan(backend, plan, assets)
  for each plan surface:
    -> backend.ensure_surface(...)
  for each pass in order:
    -> backend.exec_scene/offscreen/composite(...)
  -> backend.readback_rgba8(final_surface,...)
  -> FrameRGBA
```

### 8.7 Pipeline orchestration APIs

#### 8.7.1 Frame and range rendering

```text
render_frame(comp, frame, backend, assets)
  -> comp.validate
  -> resolve_layout_offsets
  -> Evaluator::eval_frame_with_layout_unchecked
  -> compile_frame_with_cache (fresh cache for call)
  -> execute_plan
  -> FrameRGBA

render_frames(comp, range, backend, assets)
  -> render_frames_with_stats(..., RenderThreading::default())
  => discard stats
  -> Vec<FrameRGBA>

render_frames_with_stats(comp, range, backend, assets, threading)
  => validate non-empty range
  -> comp.validate
  -> resolve_layout_offsets
  -> allocate rolling CompileCache (sequential path)
  if sequential:
    for frame:
      eval_unchecked -> compile_with_cache -> execute
    -> frames + stats(rendered==total)
  if parallel:
    -> backend.worker_render_settings() required
    -> build_thread_pool(threads)
    -> per chunk render_chunk_parallel_cpu
    -> aggregate stats and ordered frames
```

#### 8.7.2 MP4 rendering

```text
render_to_mp4(comp, out, opts, backend, assets)
  -> render_to_mp4_with_stats(...)
  => discard stats

render_to_mp4_with_stats(comp, out, opts, backend, assets)
  -> comp.validate
  => validate range and integer fps requirement
  -> is_ffmpeg_on_path check
  -> build_audio_manifest(comp, assets, range)
  -> if manifest has segments:
       mix_manifest -> write_mix_to_f32le_file(temp)
       build AudioInputConfig
  -> build EncodeConfig
  -> FfmpegEncoder::new(cfg,bg)
  -> resolve_layout_offsets
  -> sequential chunk path: render_chunk_sequential + encode each frame
  -> parallel chunk path: render_chunk_parallel_cpu_unique + encode by frame_to_unique mapping
  -> FfmpegEncoder::finish
  -> TempFileGuard drop removes temp audio file
  -> RenderStats
```

### 8.8 Audio APIs

```text
build_audio_manifest(comp, assets, range)
  => validate non-empty range
  for each track clip intersecting range:
    if Audio asset:
      -> push_audio_segment
    if Video asset:
      -> push_video_audio_segment (only when prepared video contains audio)
  => total_samples via frame_to_sample
  -> AudioManifest

mix_manifest(manifest)
  => allocate output interleaved f32 buffer
  for each segment:
    for each timeline sample:
      => map to floating source sample index by source_start_sec + rel_sec*playback_rate
      => linearly interpolate between adjacent source frames
      => apply fade_gain * volume
      => add into output channels
  => clamp output [-1,1]
  -> Vec<f32>

frame_to_sample(frame_delta, fps, sample_rate)
  => rational conversion with rounding ((num + den/2)/den)

write_mix_to_f32le_file(samples, out_path)
  -> create parent dir
  => convert each f32 to little-endian bytes
  -> fs::write
```

### 8.9 Encoding APIs

```text
default_mp4_config(out,width,height,fps)
  => EncodeConfig { overwrite=true, audio=None, ... }

is_ffmpeg_on_path()
  -> run `ffmpeg -version`
  => status.success

ensure_parent_dir(path)
  -> create_dir_all(path.parent) when present

EncodeConfig::validate()
  => width/height non-zero and even
  => fps non-zero
  => audio sample_rate/channels valid when audio present

EncodeConfig::with_out_path(path)
  => clone-like setter returning updated config

FfmpegEncoder::new(cfg,bg)
  -> cfg.validate
  -> ensure_parent_dir(cfg.out_path)
  => enforce overwrite policy
  -> is_ffmpeg_on_path check
  -> build ffmpeg command args
  -> spawn process and take stdin
  -> spawn stderr drain thread
  -> encoder with scratch frame buffer

FfmpegEncoder::encode_frame(frame)
  => dimension and byte-size checks
  -> flatten_to_opaque_rgba8(scratch, frame.data, frame.premultiplied, bg)
  -> stdin.write_all(scratch)

FfmpegEncoder::finish()
  => close stdin
  -> child.wait()
  -> join stderr drain thread
  => status success check
```

### 8.10 Misc exported APIs

```text
decode_image(bytes)
  -> image decode + premultiply

parse_svg(bytes)
  -> usvg parse

parse_transition(spec)
  -> parse_transition_kind_params

parse_effect(inst)
  -> effect parser

normalize_effects(effects)
  -> effect normalizer

compile_frame(comp,eval,assets)
  -> render plan compiler (see 8.5.4)

fingerprint_eval(eval)
  -> stable frame fingerprint (see 8.5.3)
```

### 8.11 CLI command call chains (non-library but operational API)

```text
`wavyte frame --in ... --frame ... --out ...`
  -> read_comp_json
  -> comp.validate
  -> create_backend
  -> PreparedAssetStore::prepare(root from input file parent)
  -> optional dump_font_diagnostics
  -> render_frame
  -> write PNG

`wavyte render --in ... --out ...`
  -> read_comp_json
  -> comp.validate
  -> create_backend
  -> PreparedAssetStore::prepare(root from input file parent)
  -> optional dump_font_diagnostics
  -> render_to_mp4(full range, default threading)
```

### 8.12 Internal-public addendum (not re-exported, still useful)

These are `pub` in source modules but not part of `lib.rs` re-export surface:

```text
svg_raster_params(tree, transform)
  => infer scale from affine coeffs
  => choose raster size + transform_adjust

rasterize_svg_to_premul_rgba8(tree,w,h)
  -> resvg render into tiny_skia pixmap
  -> Vec<u8>

parse_transition_kind_params(kind, params)
  => parse transition aliasing/params

over/crossfade/over_in_place/crossfade_over_in_place/wipe_over_in_place
  => premultiplied compositing primitives used by CpuBackend::exec_composite

blur_rgba8_premul(...)
  => separable gaussian blur used by CpuBackend::exec_offscreen
```

## 9. Raw symbol appendix (generated from `wavyte-core/src/` + `wavyte-cli/src/`)

The following section is a direct inventory generated from:
- `rg -n "^(pub\s+)?(type|struct|enum|trait)\s+|^impl\b|^\s*(pub\s+)?fn\s" wavyte-core/src wavyte-cli/src -g '*.rs'`

It is included so this document is auditable for coverage.

```text
wavyte-cli/src/main.rs:13:struct Cli {
wavyte-cli/src/main.rs:19:enum Command {
wavyte-cli/src/main.rs:27:struct FrameArgs {
wavyte-cli/src/main.rs:54:struct RenderArgs {
wavyte-cli/src/main.rs:77:enum BackendChoice {
wavyte-cli/src/main.rs:81:fn main() -> anyhow::Result<()> {
wavyte-cli/src/main.rs:89:fn read_comp_json(path: &Path) -> anyhow::Result<wavyte::Composition> {
wavyte-cli/src/main.rs:97:fn make_backend(
wavyte-cli/src/main.rs:108:fn cmd_frame(args: FrameArgs) -> anyhow::Result<()> {
wavyte-cli/src/main.rs:151:fn cmd_render(args: RenderArgs) -> anyhow::Result<()> {
wavyte-cli/src/main.rs:180:fn dump_font_diagnostics(
wavyte-cli/src/main.rs:240:fn sha256_hex(bytes: &[u8]) -> String {
wavyte-cli/src/main.rs:249:fn count_svg_text_nodes(group: &usvg::Group) -> usize {
wavyte-core/src/audio/mix.rs:12:pub struct AudioSegment {
wavyte-core/src/audio/mix.rs:27:pub struct AudioManifest {
wavyte-core/src/audio/mix.rs:34:pub fn build_audio_manifest(
wavyte-core/src/audio/mix.rs:93:pub fn mix_manifest(manifest: &AudioManifest) -> Vec<f32> {
wavyte-core/src/audio/mix.rs:161:pub fn write_mix_to_f32le_file(samples_interleaved: &[f32], out_path: &Path) -> WavyteResult<()> {
wavyte-core/src/audio/mix.rs:183:fn fade_gain(seg: &AudioSegment, rel_sec: f64, seg_len_samples: u64, sample_rate: u32) -> f32 {
wavyte-core/src/audio/mix.rs:198:fn push_audio_segment(
wavyte-core/src/audio/mix.rs:236:fn push_video_audio_segment(
wavyte-core/src/audio/mix.rs:278:fn push_segment_common(
wavyte-core/src/audio/mix.rs:321:pub fn frame_to_sample(frame_delta: u64, fps: Fps, sample_rate: u32) -> u64 {
wavyte-core/src/audio/mix.rs:327:fn intersect_ranges(a: FrameRange, b: FrameRange) -> Option<FrameRange> {
wavyte-core/src/audio/mix.rs:341:    fn frame_to_sample_uses_rational_fps() {
wavyte-core/src/audio/mix.rs:348:    fn mix_applies_overlap_and_fades() {
wavyte-core/src/animation/ease.rs:2:pub enum Ease {
wavyte-core/src/animation/ease.rs:12:impl Ease {
wavyte-core/src/animation/ease.rs:13:    pub fn apply(self, t: f64) -> f64 {
wavyte-core/src/animation/ease.rs:44:    fn endpoints_are_stable() {
wavyte-core/src/animation/ease.rs:60:    fn monotonic_spot_check() {
wavyte-core/src/render/cpu.rs:13:pub struct CpuBackend {
wavyte-core/src/render/cpu.rs:22:struct CpuSurface {
wavyte-core/src/render/cpu.rs:28:struct VideoFrameDecoder {
wavyte-core/src/render/cpu.rs:36:impl VideoFrameDecoder {
wavyte-core/src/render/cpu.rs:37:    fn new(info: std::sync::Arc<media::VideoSourceInfo>) -> Self {
wavyte-core/src/render/cpu.rs:57:    fn decode_at(&mut self, source_time_s: f64) -> WavyteResult<vello_cpu::Image> {
wavyte-core/src/render/cpu.rs:78:    fn key_for_time(&self, source_time_s: f64) -> u64 {
wavyte-core/src/render/cpu.rs:82:    fn prefetch_for_key(&mut self, key_ms: u64) -> WavyteResult<()> {
wavyte-core/src/render/cpu.rs:108:    fn rgba_to_image(&self, rgba: &[u8]) -> WavyteResult<vello_cpu::Image> {
wavyte-core/src/render/cpu.rs:116:    fn insert_frame(&mut self, key: u64, image: vello_cpu::Image) {
wavyte-core/src/render/cpu.rs:126:    fn touch(&mut self, key: u64) {
wavyte-core/src/render/cpu.rs:134:impl CpuBackend {
wavyte-core/src/render/cpu.rs:135:    pub fn new(settings: RenderSettings) -> Self {
wavyte-core/src/render/cpu.rs:147:impl PassBackend for CpuBackend {
wavyte-core/src/render/cpu.rs:148:    fn ensure_surface(&mut self, id: SurfaceId, desc: &SurfaceDesc) -> WavyteResult<()> {
wavyte-core/src/render/cpu.rs:195:    fn exec_scene(
wavyte-core/src/render/cpu.rs:221:    fn exec_offscreen(
wavyte-core/src/render/cpu.rs:266:    fn exec_composite(
wavyte-core/src/render/cpu.rs:351:    fn readback_rgba8(
wavyte-core/src/render/cpu.rs:376:impl RenderBackend for CpuBackend {
wavyte-core/src/render/cpu.rs:377:    fn worker_render_settings(&self) -> Option<RenderSettings> {
wavyte-core/src/render/cpu.rs:382:fn premul_rgba8(r: u8, g: u8, b: u8, a: u8) -> [u8; 4] {
wavyte-core/src/render/cpu.rs:388:fn clear_pixmap(pixmap: &mut vello_cpu::Pixmap, rgba: [u8; 4]) {
wavyte-core/src/render/cpu.rs:395:fn draw_op(
wavyte-core/src/render/cpu.rs:544:fn affine_to_cpu(a: crate::core::Affine) -> vello_cpu::kurbo::Affine {
wavyte-core/src/render/cpu.rs:548:fn point_to_cpu(p: crate::core::Point) -> vello_cpu::kurbo::Point {
wavyte-core/src/render/cpu.rs:552:fn bezpath_to_cpu(path: &crate::core::BezPath) -> vello_cpu::kurbo::BezPath {
wavyte-core/src/render/cpu.rs:570:fn image_premul_bytes_to_pixmap(
wavyte-core/src/render/cpu.rs:608:fn image_paint_size(image: &vello_cpu::Image) -> WavyteResult<(f64, f64)> {
wavyte-core/src/render/cpu.rs:617:impl CpuBackend {
wavyte-core/src/render/cpu.rs:618:    fn image_paint_for(
wavyte-core/src/render/cpu.rs:643:    fn font_for_text_asset(
wavyte-core/src/render/cpu.rs:663:    fn svg_paint_for(
wavyte-core/src/render/cpu.rs:696:    fn video_paint_for(
wavyte-core/src/animation/proc.rs:8:pub struct Procedural<T> {
wavyte-core/src/animation/proc.rs:14:impl<T> Procedural<T> {
wavyte-core/src/animation/proc.rs:15:    pub fn new(kind: ProceduralKind) -> Self {
wavyte-core/src/animation/proc.rs:23:pub trait ProcValue: Sized {
wavyte-core/src/animation/proc.rs:24:    fn from_procedural(kind: &ProceduralKind, ctx: SampleCtx) -> WavyteResult<Self>;
wavyte-core/src/animation/proc.rs:27:impl<T> Procedural<T>
wavyte-core/src/animation/proc.rs:31:    pub fn sample(&self, ctx: SampleCtx) -> WavyteResult<T> {
wavyte-core/src/animation/proc.rs:38:pub enum ProceduralKind {
wavyte-core/src/animation/proc.rs:44:pub enum ProcScalar {
wavyte-core/src/animation/proc.rs:70:pub struct Rng64 {
wavyte-core/src/animation/proc.rs:74:impl Rng64 {
wavyte-core/src/animation/proc.rs:75:    pub fn new(seed: u64) -> Self {
wavyte-core/src/animation/proc.rs:79:    pub fn next_u64(&mut self) -> u64 {
wavyte-core/src/animation/proc.rs:88:    pub fn next_f64_01(&mut self) -> f64 {
wavyte-core/src/animation/proc.rs:95:fn noise01(seed: u64, x: u64) -> f64 {
wavyte-core/src/animation/proc.rs:100:fn sample_scalar(s: &ProcScalar, fps: Fps, frame: u64, seed: u64) -> f64 {
wavyte-core/src/animation/proc.rs:159:impl ProcValue for f64 {
wavyte-core/src/animation/proc.rs:160:    fn from_procedural(kind: &ProceduralKind, ctx: SampleCtx) -> WavyteResult<Self> {
wavyte-core/src/animation/proc.rs:170:impl ProcValue for f32 {
wavyte-core/src/animation/proc.rs:171:    fn from_procedural(kind: &ProceduralKind, ctx: SampleCtx) -> WavyteResult<Self> {
wavyte-core/src/animation/proc.rs:176:impl ProcValue for Vec2 {
wavyte-core/src/animation/proc.rs:177:    fn from_procedural(kind: &ProceduralKind, ctx: SampleCtx) -> WavyteResult<Self> {
wavyte-core/src/animation/proc.rs:190:impl ProcValue for Transform2D {
wavyte-core/src/animation/proc.rs:191:    fn from_procedural(_kind: &ProceduralKind, _ctx: SampleCtx) -> WavyteResult<Self> {
wavyte-core/src/animation/proc.rs:198:impl ProcValue for crate::core::Rgba8Premul {
wavyte-core/src/animation/proc.rs:199:    fn from_procedural(_kind: &ProceduralKind, _ctx: SampleCtx) -> WavyteResult<Self> {
wavyte-core/src/animation/proc.rs:211:    fn ctx(frame: u64, seed: u64) -> SampleCtx {
wavyte-core/src/animation/proc.rs:221:    fn rng_is_deterministic() {
wavyte-core/src/animation/proc.rs:230:    fn noise_is_bounded_and_deterministic() {
wavyte-core/src/animation/proc.rs:247:    fn envelope_basic_boundaries() {
wavyte-core/src/render/backend.rs:13:pub struct FrameRGBA {
wavyte-core/src/render/backend.rs:26:pub trait RenderBackend: PassBackend {
wavyte-core/src/render/backend.rs:27:    fn render_plan(
wavyte-core/src/render/backend.rs:35:    fn worker_render_settings(&self) -> Option<RenderSettings> {
wavyte-core/src/render/backend.rs:44:pub enum BackendKind {
wavyte-core/src/render/backend.rs:50:pub struct RenderSettings {
wavyte-core/src/render/backend.rs:58:pub fn create_backend(
wavyte-core/src/animation/ops.rs:3:pub fn delay<T>(inner: Anim<T>, by_frames: u64) -> Anim<T> {
wavyte-core/src/animation/ops.rs:10:pub fn speed<T>(inner: Anim<T>, factor: f64) -> Anim<T> {
wavyte-core/src/animation/ops.rs:17:pub fn reverse<T>(inner: Anim<T>, duration_frames: u64) -> Anim<T> {
wavyte-core/src/animation/ops.rs:24:pub fn loop_<T>(inner: Anim<T>, period_frames: u64, mode: LoopMode) -> Anim<T> {
wavyte-core/src/animation/ops.rs:32:pub fn mix<T>(a: Anim<T>, b: Anim<T>, t: Anim<f64>) -> Anim<T> {
wavyte-core/src/animation/ops.rs:40:pub fn sequence(a: Anim<f64>, a_len: u64, b: Anim<f64>) -> Anim<f64> {
wavyte-core/src/animation/ops.rs:63:pub fn stagger(mut anims: Vec<(u64, Anim<f64>)>) -> Anim<f64> {
wavyte-core/src/animation/ops.rs:83:    fn ctx(frame: u64) -> SampleCtx {
wavyte-core/src/animation/ops.rs:93:    fn sequence_switches_at_boundary() {
wavyte-core/src/render/passes.rs:8:pub trait PassBackend {
wavyte-core/src/render/passes.rs:9:    fn ensure_surface(&mut self, id: SurfaceId, desc: &SurfaceDesc) -> WavyteResult<()>;
wavyte-core/src/render/passes.rs:11:    fn exec_scene(&mut self, pass: &ScenePass, assets: &PreparedAssetStore) -> WavyteResult<()>;
wavyte-core/src/render/passes.rs:13:    fn exec_offscreen(
wavyte-core/src/render/passes.rs:19:    fn exec_composite(
wavyte-core/src/render/passes.rs:25:    fn readback_rgba8(
wavyte-core/src/render/passes.rs:33:pub fn execute_plan<B: PassBackend + ?Sized>(
wavyte-core/src/render/passes.rs:73:        fn ensure_surface(&mut self, _id: SurfaceId, _desc: &SurfaceDesc) -> WavyteResult<()> {
wavyte-core/src/render/passes.rs:78:        fn exec_scene(
wavyte-core/src/render/passes.rs:87:        fn exec_offscreen(
wavyte-core/src/render/passes.rs:96:        fn exec_composite(
wavyte-core/src/render/passes.rs:105:        fn readback_rgba8(
wavyte-core/src/render/passes.rs:122:    fn execute_plan_calls_in_expected_order() {
wavyte-core/src/render/passes.rs:196:    fn execute_plan_returns_final_frame() {
wavyte-core/src/animation/anim.rs:9:pub struct SampleCtx {
wavyte-core/src/animation/anim.rs:16:pub trait Lerp: Sized {
wavyte-core/src/animation/anim.rs:17:    fn lerp(a: &Self, b: &Self, t: f64) -> Self;
wavyte-core/src/animation/anim.rs:20:impl Lerp for f64 {
wavyte-core/src/animation/anim.rs:21:    fn lerp(a: &Self, b: &Self, t: f64) -> Self {
wavyte-core/src/animation/anim.rs:26:impl Lerp for f32 {
wavyte-core/src/animation/anim.rs:27:    fn lerp(a: &Self, b: &Self, t: f64) -> Self {
wavyte-core/src/animation/anim.rs:32:impl Lerp for Vec2 {
wavyte-core/src/animation/anim.rs:33:    fn lerp(a: &Self, b: &Self, t: f64) -> Self {
wavyte-core/src/animation/anim.rs:38:impl Lerp for Transform2D {
wavyte-core/src/animation/anim.rs:39:    fn lerp(a: &Self, b: &Self, t: f64) -> Self {
wavyte-core/src/animation/anim.rs:49:impl Lerp for crate::core::Rgba8Premul {
wavyte-core/src/animation/anim.rs:50:    fn lerp(a: &Self, b: &Self, t: f64) -> Self {
wavyte-core/src/animation/anim.rs:51:        fn lerp_u8(a: u8, b: u8, t: f64) -> u8 {
wavyte-core/src/animation/anim.rs:67:pub enum Anim<T> {
wavyte-core/src/animation/anim.rs:73:impl<T> Anim<T>
wavyte-core/src/animation/anim.rs:77:    pub fn constant(value: T) -> Self {
wavyte-core/src/animation/anim.rs:89:    pub fn sample(&self, ctx: SampleCtx) -> WavyteResult<T> {
wavyte-core/src/animation/anim.rs:97:    pub fn validate(&self) -> WavyteResult<()> {
wavyte-core/src/animation/anim.rs:107:pub struct Keyframes<T> {
wavyte-core/src/animation/anim.rs:113:impl<T> Keyframes<T>
wavyte-core/src/animation/anim.rs:117:    pub fn validate(&self) -> WavyteResult<()> {
wavyte-core/src/animation/anim.rs:131:    pub fn sample(&self, ctx: SampleCtx) -> WavyteResult<T> {
wavyte-core/src/animation/anim.rs:166:pub struct Keyframe<T> {
wavyte-core/src/animation/anim.rs:173:pub enum InterpMode {
wavyte-core/src/animation/anim.rs:179:pub enum Expr<T> {
wavyte-core/src/animation/anim.rs:205:pub enum LoopMode {
wavyte-core/src/animation/anim.rs:210:impl<T> Expr<T>
wavyte-core/src/animation/anim.rs:214:    pub fn validate(&self) -> WavyteResult<()> {
wavyte-core/src/animation/anim.rs:247:    pub fn sample(&self, ctx: SampleCtx) -> WavyteResult<T> {
wavyte-core/src/animation/anim.rs:248:        fn with_clip_local(mut ctx: SampleCtx, clip_local: FrameIndex) -> SampleCtx {
wavyte-core/src/animation/anim.rs:325:    fn ctx(frame: u64) -> SampleCtx {
wavyte-core/src/animation/anim.rs:335:    fn keyframes_hold_is_constant_between_keys() {
wavyte-core/src/animation/anim.rs:357:    fn keyframes_linear_interpolates() {
wavyte-core/src/animation/anim.rs:378:    fn expr_reverse_maps_frames() {
wavyte-core/src/encode/ffmpeg.rs:15:pub struct EncodeConfig {
wavyte-core/src/encode/ffmpeg.rs:25:pub struct AudioInputConfig {
wavyte-core/src/encode/ffmpeg.rs:31:impl EncodeConfig {
wavyte-core/src/encode/ffmpeg.rs:33:    pub fn validate(&self) -> WavyteResult<()> {
wavyte-core/src/encode/ffmpeg.rs:64:    pub fn with_out_path(mut self, out_path: impl Into<PathBuf>) -> Self {
wavyte-core/src/encode/ffmpeg.rs:71:pub fn default_mp4_config(
wavyte-core/src/encode/ffmpeg.rs:88:pub fn is_ffmpeg_on_path() -> bool {
wavyte-core/src/encode/ffmpeg.rs:99:pub fn ensure_parent_dir(path: &Path) -> WavyteResult<()> {
wavyte-core/src/encode/ffmpeg.rs:113:pub struct FfmpegEncoder {
wavyte-core/src/encode/ffmpeg.rs:122:impl FfmpegEncoder {
wavyte-core/src/encode/ffmpeg.rs:124:    pub fn new(cfg: EncodeConfig, bg_rgba: [u8; 4]) -> WavyteResult<Self> {
wavyte-core/src/encode/ffmpeg.rs:237:    pub fn encode_frame(&mut self, frame: &FrameRGBA) -> WavyteResult<()> {
wavyte-core/src/encode/ffmpeg.rs:273:    pub fn finish(mut self) -> WavyteResult<()> {
wavyte-core/src/encode/ffmpeg.rs:300:fn flatten_to_opaque_rgba8(
wavyte-core/src/encode/ffmpeg.rs:349:fn mul_div255(x: u16, y: u16) -> u16 {
wavyte-core/src/encode/ffmpeg.rs:358:    fn config_validation_catches_bad_values() {
wavyte-core/src/encode/ffmpeg.rs:400:    fn flatten_premul_over_black_produces_expected_rgb() {
wavyte-core/src/encode/ffmpeg.rs:409:    fn flatten_straight_over_black_produces_expected_rgb() {
wavyte-core/src/compile/fingerprint.rs:4:pub struct FrameFingerprint {
wavyte-core/src/compile/fingerprint.rs:9:pub fn fingerprint_eval(eval: &EvaluatedGraph) -> FrameFingerprint {
wavyte-core/src/compile/fingerprint.rs:69:fn write_json_value_pair(a: &mut Fnv1a64, b: &mut Fnv1a64, v: &serde_json::Value) {
wavyte-core/src/compile/fingerprint.rs:104:fn write_u8_pair(a: &mut Fnv1a64, b: &mut Fnv1a64, v: u8) {
wavyte-core/src/compile/fingerprint.rs:109:fn write_u64_pair(a: &mut Fnv1a64, b: &mut Fnv1a64, v: u64) {
wavyte-core/src/compile/fingerprint.rs:114:fn write_i64_pair(a: &mut Fnv1a64, b: &mut Fnv1a64, v: i64) {
wavyte-core/src/compile/fingerprint.rs:118:fn write_str_pair(a: &mut Fnv1a64, b: &mut Fnv1a64, s: &str) {
wavyte-core/src/compile/fingerprint.rs:129:    fn comp_with_opacity(opacity: f64) -> Composition {
wavyte-core/src/compile/fingerprint.rs:172:    fn fingerprint_is_deterministic_for_same_eval() {
wavyte-core/src/compile/fingerprint.rs:181:    fn fingerprint_changes_when_scene_changes() {
wavyte-core/src/render/pipeline.rs:27:pub fn render_frame(
wavyte-core/src/render/pipeline.rs:44:pub fn render_frames(
wavyte-core/src/render/pipeline.rs:55:pub struct RenderThreading {
wavyte-core/src/render/pipeline.rs:62:impl Default for RenderThreading {
wavyte-core/src/render/pipeline.rs:63:    fn default() -> Self {
wavyte-core/src/render/pipeline.rs:74:pub struct RenderStats {
wavyte-core/src/render/pipeline.rs:80:pub fn render_frames_with_stats(
wavyte-core/src/render/pipeline.rs:146:pub struct RenderToMp4Opts {
wavyte-core/src/render/pipeline.rs:157:impl Default for RenderToMp4Opts {
wavyte-core/src/render/pipeline.rs:158:    fn default() -> Self {
wavyte-core/src/render/pipeline.rs:179:pub fn render_to_mp4(
wavyte-core/src/render/pipeline.rs:190:pub fn render_to_mp4_with_stats(
wavyte-core/src/render/pipeline.rs:327:fn render_chunk_sequential(
wavyte-core/src/render/pipeline.rs:353:struct ChunkParallelOut {
wavyte-core/src/render/pipeline.rs:359:fn render_chunk_parallel_cpu_unique(
wavyte-core/src/render/pipeline.rs:440:fn render_chunk_parallel_cpu(
wavyte-core/src/render/pipeline.rs:492:fn build_thread_pool(threads: Option<usize>) -> WavyteResult<rayon::ThreadPool> {
wavyte-core/src/render/pipeline.rs:510:fn normalized_chunk_size(chunk_size: usize) -> u64 {
wavyte-core/src/render/pipeline.rs:518:struct TempFileGuard(Option<std::path::PathBuf>);
wavyte-core/src/render/pipeline.rs:520:impl Drop for TempFileGuard {
wavyte-core/src/render/pipeline.rs:521:    fn drop(&mut self) {
wavyte-core/src/assets/store.rs:18:pub struct PreparedImage {
wavyte-core/src/assets/store.rs:25:pub struct PreparedSvg {
wavyte-core/src/assets/store.rs:30:pub struct TextBrushRgba8 {
wavyte-core/src/assets/store.rs:38:pub struct PreparedText {
wavyte-core/src/assets/store.rs:44:impl std::fmt::Debug for PreparedText {
wavyte-core/src/assets/store.rs:45:    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
wavyte-core/src/assets/store.rs:55:pub struct PreparedPath {
wavyte-core/src/assets/store.rs:60:pub struct PreparedAudio {
wavyte-core/src/assets/store.rs:67:pub struct PreparedVideo {
wavyte-core/src/assets/store.rs:73:pub enum PreparedAsset {
wavyte-core/src/assets/store.rs:83:pub struct AssetId(pub(crate) u64);
wavyte-core/src/assets/store.rs:85:impl AssetId {
wavyte-core/src/assets/store.rs:86:    pub fn from_u64(raw: u64) -> Self {
wavyte-core/src/assets/store.rs:90:    pub fn as_u64(self) -> u64 {
wavyte-core/src/assets/store.rs:96:pub struct AssetKey {
wavyte-core/src/assets/store.rs:101:impl AssetKey {
wavyte-core/src/assets/store.rs:102:    pub fn new(norm_path: String, mut params: Vec<(String, String)>) -> Self {
wavyte-core/src/assets/store.rs:109:pub struct PreparedAssetStore {
wavyte-core/src/assets/store.rs:115:impl PreparedAssetStore {
wavyte-core/src/assets/store.rs:116:    pub fn prepare(comp: &model::Composition, root: impl Into<PathBuf>) -> WavyteResult<Self> {
wavyte-core/src/assets/store.rs:207:    pub fn root(&self) -> &Path {
wavyte-core/src/assets/store.rs:211:    pub fn id_for_key(&self, key: &str) -> WavyteResult<AssetId> {
wavyte-core/src/assets/store.rs:218:    pub fn get(&self, id: AssetId) -> WavyteResult<&PreparedAsset> {
wavyte-core/src/assets/store.rs:224:    fn key_for(&self, asset: &model::Asset) -> WavyteResult<(u8, AssetKey)> {
wavyte-core/src/assets/store.rs:272:    fn hash_id_for_key(kind_tag: u8, key: &AssetKey) -> AssetId {
wavyte-core/src/assets/store.rs:286:    fn read_bytes(&self, norm_path: &str) -> WavyteResult<Vec<u8>> {
wavyte-core/src/assets/store.rs:294:fn parse_svg_with_options(root: &Path, norm_path: &str, bytes: &[u8]) -> WavyteResult<PreparedSvg> {
wavyte-core/src/assets/store.rs:313:fn build_svg_fontdb(
wavyte-core/src/assets/store.rs:331:fn load_fonts_from_dir(db: &mut usvg::fontdb::Database, dir: &Path) {
wavyte-core/src/assets/store.rs:352:fn make_svg_font_resolver() -> usvg::FontResolver<'static> {
wavyte-core/src/assets/store.rs:407:pub fn normalize_rel_path(source: &str) -> WavyteResult<String> {
wavyte-core/src/assets/store.rs:436:fn parse_svg_path(d: &str) -> WavyteResult<BezPath> {
wavyte-core/src/assets/store.rs:447:pub struct TextLayoutEngine {
wavyte-core/src/assets/store.rs:453:impl Default for TextLayoutEngine {
wavyte-core/src/assets/store.rs:454:    fn default() -> Self {
wavyte-core/src/assets/store.rs:459:impl TextLayoutEngine {
wavyte-core/src/assets/store.rs:460:    pub fn new() -> Self {
wavyte-core/src/assets/store.rs:468:    pub fn last_family_name(&self) -> Option<String> {
wavyte-core/src/assets/store.rs:472:    pub fn layout_plain(
wavyte-core/src/assets/store.rs:535:    fn normalize_path_slash_normalization() {
wavyte-core/src/assets/store.rs:543:    fn asset_id_stability_same_input() {
wavyte-core/src/assets/store.rs:559:    fn prepare_path_assets_without_external_io() {
wavyte-core/src/assets/store.rs:590:    fn text_layout_smoke_with_local_font_if_present() {
wavyte-core/src/assets/store.rs:616:    fn prepare_single_image_asset() {
wavyte-core/src/compile/plan.rs:14:struct EffectCacheKey {
wavyte-core/src/compile/plan.rs:20:struct TransitionCacheKey {
wavyte-core/src/compile/plan.rs:62:pub struct RenderPlan {
wavyte-core/src/compile/plan.rs:71:pub enum Pass {
wavyte-core/src/compile/plan.rs:79:pub struct ScenePass {
wavyte-core/src/compile/plan.rs:87:pub struct SurfaceId(pub u32);
wavyte-core/src/compile/plan.rs:91:pub enum PixelFormat {
wavyte-core/src/compile/plan.rs:97:pub struct SurfaceDesc {
wavyte-core/src/compile/plan.rs:105:pub struct OffscreenPass {
wavyte-core/src/compile/plan.rs:113:pub struct CompositePass {
wavyte-core/src/compile/plan.rs:120:pub enum CompositeOp {
wavyte-core/src/compile/plan.rs:141:pub enum DrawOp {
wavyte-core/src/compile/plan.rs:181:pub fn compile_frame(
wavyte-core/src/compile/plan.rs:409:fn parse_effect_cached(
wavyte-core/src/compile/plan.rs:442:fn parse_transition_cached(
wavyte-core/src/compile/plan.rs:611:    fn store_for(comp: &Composition) -> PreparedAssetStore {
wavyte-core/src/compile/plan.rs:616:    fn compile_path_emits_fillpath_without_asset_cache() {
wavyte-core/src/compile/plan.rs:681:    fn compile_applies_inline_effects_to_opacity_and_transform() {
wavyte-core/src/compile/plan.rs:754:    fn compile_emits_offscreen_blur_pass_and_composites_blurred_surface() {
wavyte-core/src/compile/plan.rs:845:    fn compile_pairs_crossfade_into_single_composite_op() {
wavyte-core/src/compile/plan.rs:931:    fn compile_pairs_wipe_into_single_composite_op() {
wavyte-core/src/compile/plan.rs:1025:    fn compile_does_not_pair_transitions_when_progress_is_not_aligned() {
wavyte-core/src/layout/solver.rs:9:pub struct LayoutOffsets {
wavyte-core/src/layout/solver.rs:13:impl LayoutOffsets {
wavyte-core/src/layout/solver.rs:14:    pub fn offset_for(&self, track_idx: usize, clip_idx: usize) -> Vec2 {
wavyte-core/src/layout/solver.rs:23:pub fn resolve_layout_offsets(
wavyte-core/src/layout/solver.rs:34:fn resolve_track_offsets(
wavyte-core/src/layout/solver.rs:107:fn intrinsic_size_for_asset_key(
wavyte-core/src/layout/solver.rs:139:fn align_offset<W: Into<f64>, C: Into<f64>, A>(container: W, content: C, align: A) -> f64
wavyte-core/src/layout/solver.rs:153:enum AlignKind {
wavyte-core/src/layout/solver.rs:159:impl From<LayoutAlignX> for AlignKind {
wavyte-core/src/layout/solver.rs:160:    fn from(value: LayoutAlignX) -> Self {
wavyte-core/src/layout/solver.rs:169:impl From<LayoutAlignY> for AlignKind {
wavyte-core/src/layout/solver.rs:170:    fn from(value: LayoutAlignY) -> Self {
wavyte-core/src/layout/solver.rs:184:    fn comp_for_layout(mode: LayoutMode) -> Composition {
wavyte-core/src/layout/solver.rs:251:    fn hstack_offsets_are_deterministic() {
wavyte-core/src/layout/solver.rs:260:    fn center_mode_centers_each_clip() {
wavyte-core/src/assets/svg_raster.rs:4:pub struct SvgRasterKey {
wavyte-core/src/assets/svg_raster.rs:18:pub fn svg_raster_params(
wavyte-core/src/assets/svg_raster.rs:22:    fn to_px(v: f32) -> WavyteResult<u32> {
wavyte-core/src/assets/svg_raster.rs:57:pub fn rasterize_svg_to_premul_rgba8(
wavyte-core/src/assets/media.rs:11:pub struct VideoSourceInfo {
wavyte-core/src/assets/media.rs:22:pub struct AudioPcm {
wavyte-core/src/assets/media.rs:28:impl VideoSourceInfo {
wavyte-core/src/assets/media.rs:29:    pub fn source_fps(&self) -> f64 {
wavyte-core/src/assets/media.rs:38:pub fn video_source_time_sec(asset: &VideoAsset, clip_local_frames: u64, fps: crate::Fps) -> f64 {
wavyte-core/src/assets/media.rs:47:pub fn audio_source_time_sec(asset: &AudioAsset, clip_local_frames: u64, fps: crate::Fps) -> f64 {
wavyte-core/src/assets/media.rs:57:pub fn probe_video(source_path: &Path) -> WavyteResult<VideoSourceInfo> {
wavyte-core/src/assets/media.rs:134:pub fn probe_video(_source_path: &Path) -> WavyteResult<VideoSourceInfo> {
wavyte-core/src/assets/media.rs:141:pub fn decode_video_frame_rgba8(
wavyte-core/src/assets/media.rs:213:pub fn decode_video_frame_rgba8(
wavyte-core/src/assets/media.rs:234:pub fn decode_audio_f32_stereo(path: &Path, sample_rate: u32) -> WavyteResult<AudioPcm> {
wavyte-core/src/assets/media.rs:294:pub fn decode_audio_f32_stereo(_path: &Path, _sample_rate: u32) -> WavyteResult<AudioPcm> {
wavyte-core/src/assets/media.rs:301:fn parse_ff_ratio(s: &str) -> Option<(u32, u32)> {
wavyte-core/src/assets/media.rs:316:    fn source_time_mapping_applies_trim_and_rate() {
wavyte-core/src/transform/non_linear.rs:4:pub fn clamp01(x: f64) -> f64 {
wavyte-core/src/assets/decode.rs:10:pub fn decode_image(bytes: &[u8]) -> WavyteResult<PreparedImage> {
wavyte-core/src/assets/decode.rs:25:pub fn parse_svg(bytes: &[u8]) -> WavyteResult<PreparedSvg> {
wavyte-core/src/assets/decode.rs:33:fn premultiply_rgba8_in_place(rgba: &mut [u8]) {
wavyte-core/src/assets/decode.rs:55:    fn decode_image_png_dimensions_and_premul() {
wavyte-core/src/assets/decode.rs:79:    fn decode_svg_parse_ok_and_err() {
wavyte-core/src/transform/linear.rs:6:pub fn lerp_vec2(a: Vec2, b: Vec2, t: f64) -> Vec2 {
wavyte-core/src/composition/dsl.rs:13:pub struct CompositionBuilder {
wavyte-core/src/composition/dsl.rs:22:impl CompositionBuilder {
wavyte-core/src/composition/dsl.rs:23:    pub fn new(fps: crate::core::Fps, canvas: Canvas, duration: FrameIndex) -> Self {
wavyte-core/src/composition/dsl.rs:34:    pub fn seed(mut self, seed: u64) -> Self {
wavyte-core/src/composition/dsl.rs:39:    pub fn asset(mut self, key: impl Into<String>, asset: Asset) -> WavyteResult<Self> {
wavyte-core/src/composition/dsl.rs:50:    pub fn track(mut self, track: Track) -> Self {
wavyte-core/src/composition/dsl.rs:55:    pub fn video_asset(
wavyte-core/src/composition/dsl.rs:63:    pub fn audio_asset(
wavyte-core/src/composition/dsl.rs:71:    pub fn build(self) -> WavyteResult<Composition> {
wavyte-core/src/composition/dsl.rs:85:pub fn video_asset(source: impl Into<String>) -> VideoAsset {
wavyte-core/src/composition/dsl.rs:98:pub fn audio_asset(source: impl Into<String>) -> AudioAsset {
wavyte-core/src/composition/dsl.rs:111:pub struct TrackBuilder {
wavyte-core/src/composition/dsl.rs:123:impl TrackBuilder {
wavyte-core/src/composition/dsl.rs:124:    pub fn new(name: impl Into<String>) -> Self {
wavyte-core/src/composition/dsl.rs:138:    pub fn z_base(mut self, z: i32) -> Self {
wavyte-core/src/composition/dsl.rs:143:    pub fn clip(mut self, clip: Clip) -> Self {
wavyte-core/src/composition/dsl.rs:148:    pub fn layout_mode(mut self, mode: crate::LayoutMode) -> Self {
wavyte-core/src/composition/dsl.rs:153:    pub fn layout_gap_px(mut self, gap: f64) -> Self {
wavyte-core/src/composition/dsl.rs:158:    pub fn layout_padding(mut self, padding: crate::Edges) -> Self {
wavyte-core/src/composition/dsl.rs:163:    pub fn layout_align(mut self, x: crate::LayoutAlignX, y: crate::LayoutAlignY) -> Self {
wavyte-core/src/composition/dsl.rs:169:    pub fn layout_grid_columns(mut self, columns: u32) -> Self {
wavyte-core/src/composition/dsl.rs:174:    pub fn build(self) -> WavyteResult<Track> {
wavyte-core/src/composition/dsl.rs:192:pub struct ClipBuilder {
wavyte-core/src/composition/dsl.rs:205:impl ClipBuilder {
wavyte-core/src/composition/dsl.rs:206:    pub fn new(id: impl Into<String>, asset_key: impl Into<String>, range: FrameRange) -> Self {
wavyte-core/src/composition/dsl.rs:221:    pub fn z_offset(mut self, z: i32) -> Self {
wavyte-core/src/composition/dsl.rs:226:    pub fn opacity(mut self, a: Anim<f64>) -> Self {
wavyte-core/src/composition/dsl.rs:231:    pub fn transform(mut self, t: Anim<Transform2D>) -> Self {
wavyte-core/src/composition/dsl.rs:236:    pub fn effect(mut self, fx: EffectInstance) -> Self {
wavyte-core/src/composition/dsl.rs:241:    pub fn transition_in(mut self, tr: TransitionSpec) -> Self {
wavyte-core/src/composition/dsl.rs:246:    pub fn transition_out(mut self, tr: TransitionSpec) -> Self {
wavyte-core/src/composition/dsl.rs:251:    pub fn build(self) -> WavyteResult<Clip> {
wavyte-core/src/composition/dsl.rs:288:    fn builders_create_expected_structure() {
wavyte-core/src/composition/dsl.rs:338:    fn duplicate_asset_key_is_rejected() {
wavyte-core/src/eval/evaluator.rs:9:pub struct EvaluatedGraph {
wavyte-core/src/eval/evaluator.rs:15:pub struct EvaluatedClipNode {
wavyte-core/src/eval/evaluator.rs:29:pub struct ResolvedEffect {
wavyte-core/src/eval/evaluator.rs:35:pub struct ResolvedTransition {
wavyte-core/src/eval/evaluator.rs:41:pub struct Evaluator;
wavyte-core/src/eval/evaluator.rs:43:impl Evaluator {
wavyte-core/src/eval/evaluator.rs:45:    pub fn eval_frame(comp: &Composition, frame: FrameIndex) -> WavyteResult<EvaluatedGraph> {
wavyte-core/src/eval/evaluator.rs:50:    pub fn eval_frame_with_layout(
wavyte-core/src/eval/evaluator.rs:66:    fn eval_frame_with_layout_impl(
wavyte-core/src/eval/evaluator.rs:111:fn eval_clip(
wavyte-core/src/eval/evaluator.rs:159:fn resolve_effect(e: &EffectInstance) -> WavyteResult<ResolvedEffect> {
wavyte-core/src/eval/evaluator.rs:169:fn resolve_transition_in(clip: &Clip, frame: FrameIndex) -> Option<ResolvedTransition> {
wavyte-core/src/eval/evaluator.rs:180:fn resolve_transition_out(clip: &Clip, frame: FrameIndex) -> Option<ResolvedTransition> {
wavyte-core/src/eval/evaluator.rs:186:enum TransitionEdge {
wavyte-core/src/eval/evaluator.rs:191:fn resolve_transition_window(
wavyte-core/src/eval/evaluator.rs:241:fn stable_hash64(seed: u64, s: &str) -> u64 {
wavyte-core/src/eval/evaluator.rs:262:    fn basic_comp(
wavyte-core/src/eval/evaluator.rs:318:    fn visibility_respects_frame_range() {
wavyte-core/src/eval/evaluator.rs:351:    fn opacity_is_clamped() {
wavyte-core/src/eval/evaluator.rs:359:    fn transition_progress_boundaries() {
wavyte-core/src/transform/affine.rs:6:pub fn compose(a: Affine, b: Affine) -> Affine {
wavyte-core/src/transform/affine.rs:11:pub fn identity() -> Affine {
wavyte-core/src/foundation/math.rs:4:impl Fnv1a64 {
wavyte-core/src/foundation/math.rs:51:    fn fnv_seeded_hash_is_stable() {
wavyte-core/src/foundation/math.rs:61:    fn mul_div255_variants_align() {
wavyte-core/src/composition/model.rs:19:pub struct Composition {
wavyte-core/src/composition/model.rs:30:pub struct Track {
wavyte-core/src/composition/model.rs:49:pub enum LayoutMode {
wavyte-core/src/composition/model.rs:59:pub struct Edges {
wavyte-core/src/composition/model.rs:71:pub enum LayoutAlignX {
wavyte-core/src/composition/model.rs:79:pub enum LayoutAlignY {
wavyte-core/src/composition/model.rs:86:fn default_layout_grid_columns() -> u32 {
wavyte-core/src/composition/model.rs:92:pub struct Clip {
wavyte-core/src/composition/model.rs:105:pub struct ClipProps {
wavyte-core/src/composition/model.rs:113:pub enum BlendMode {
wavyte-core/src/composition/model.rs:120:pub enum Asset {
wavyte-core/src/composition/model.rs:130:pub struct TextAsset {
wavyte-core/src/composition/model.rs:140:fn default_text_color_rgba8() -> [u8; 4] {
wavyte-core/src/composition/model.rs:145:pub struct SvgAsset {
wavyte-core/src/composition/model.rs:150:pub struct PathAsset {
wavyte-core/src/composition/model.rs:155:pub struct ImageAsset {
wavyte-core/src/composition/model.rs:160:pub struct VideoAsset {
wavyte-core/src/composition/model.rs:179:pub struct AudioAsset {
wavyte-core/src/composition/model.rs:197:fn default_playback_rate() -> f64 {
wavyte-core/src/composition/model.rs:201:fn default_volume() -> f64 {
wavyte-core/src/composition/model.rs:206:pub struct EffectInstance {
wavyte-core/src/composition/model.rs:213:pub struct TransitionSpec {
wavyte-core/src/composition/model.rs:221:impl Composition {
wavyte-core/src/composition/model.rs:222:    pub fn validate(&self) -> WavyteResult<()> {
wavyte-core/src/composition/model.rs:352:fn validate_rel_source(source: &str, field: &str) -> WavyteResult<()> {
wavyte-core/src/composition/model.rs:374:fn validate_media_controls(
wavyte-core/src/composition/model.rs:418:impl TransitionSpec {
wavyte-core/src/composition/model.rs:419:    pub fn validate(&self) -> WavyteResult<()> {
wavyte-core/src/composition/model.rs:442:    fn basic_comp() -> Composition {
wavyte-core/src/composition/model.rs:502:    fn json_roundtrip() {
wavyte-core/src/composition/model.rs:511:    fn validate_rejects_missing_asset() {
wavyte-core/src/composition/model.rs:518:    fn validate_rejects_out_of_bounds_range() {
wavyte-core/src/composition/model.rs:528:    fn validate_rejects_bad_fps() {
wavyte-core/src/composition/model.rs:535:    fn media_assets_serde_defaults_and_validation() {
wavyte-core/src/composition/model.rs:560:    fn media_validation_rejects_non_positive_playback_rate() {
wavyte-core/src/foundation/core.rs:8:pub struct FrameIndex(pub u64);
wavyte-core/src/foundation/core.rs:11:pub struct FrameRange {
wavyte-core/src/foundation/core.rs:16:impl FrameRange {
wavyte-core/src/foundation/core.rs:17:    pub fn new(start: FrameIndex, end: FrameIndex) -> WavyteResult<Self> {
wavyte-core/src/foundation/core.rs:24:    pub fn len_frames(self) -> u64 {
wavyte-core/src/foundation/core.rs:28:    pub fn is_empty(self) -> bool {
wavyte-core/src/foundation/core.rs:32:    pub fn contains(self, f: FrameIndex) -> bool {
wavyte-core/src/foundation/core.rs:36:    pub fn clamp(self, f: FrameIndex) -> FrameIndex {
wavyte-core/src/foundation/core.rs:44:    pub fn shift(self, delta: i64) -> Self {
wavyte-core/src/foundation/core.rs:45:        fn shift_idx(v: u64, delta: i64) -> u64 {
wavyte-core/src/foundation/core.rs:61:pub struct Fps {
wavyte-core/src/foundation/core.rs:66:impl Fps {
wavyte-core/src/foundation/core.rs:67:    pub fn new(num: u32, den: u32) -> WavyteResult<Self> {
wavyte-core/src/foundation/core.rs:77:    pub fn as_f64(self) -> f64 {
wavyte-core/src/foundation/core.rs:81:    pub fn frame_duration_secs(self) -> f64 {
wavyte-core/src/foundation/core.rs:85:    pub fn frames_to_secs(self, frames: u64) -> f64 {
wavyte-core/src/foundation/core.rs:89:    pub fn secs_to_frames_floor(self, secs: f64) -> u64 {
wavyte-core/src/foundation/core.rs:95:pub struct Canvas {
wavyte-core/src/foundation/core.rs:102:pub struct Rgba8Premul {
wavyte-core/src/foundation/core.rs:109:impl Rgba8Premul {
wavyte-core/src/foundation/core.rs:110:    pub fn transparent() -> Self {
wavyte-core/src/foundation/core.rs:119:    pub fn from_straight_rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
wavyte-core/src/foundation/core.rs:120:        fn premul(c: u8, a: u8) -> u8 {
wavyte-core/src/foundation/core.rs:136:pub struct Transform2D {
wavyte-core/src/foundation/core.rs:143:impl Default for Transform2D {
wavyte-core/src/foundation/core.rs:144:    fn default() -> Self {
wavyte-core/src/foundation/core.rs:154:impl Transform2D {
wavyte-core/src/foundation/core.rs:155:    pub fn to_affine(self) -> kurbo::Affine {
wavyte-core/src/foundation/core.rs:173:    fn frame_range_contains_boundaries() {
wavyte-core/src/foundation/core.rs:182:    fn fps_frames_secs_roundtrip_floor() {
wavyte-core/src/foundation/core.rs:189:    fn transform_to_affine_identity_and_translation() {
wavyte-core/src/foundation/error.rs:1:pub type WavyteResult<T> = Result<T, WavyteError>;
wavyte-core/src/foundation/error.rs:4:pub enum WavyteError {
wavyte-core/src/foundation/error.rs:21:impl WavyteError {
wavyte-core/src/foundation/error.rs:22:    pub fn validation(msg: impl Into<String>) -> Self {
wavyte-core/src/foundation/error.rs:26:    pub fn animation(msg: impl Into<String>) -> Self {
wavyte-core/src/foundation/error.rs:30:    pub fn evaluation(msg: impl Into<String>) -> Self {
wavyte-core/src/foundation/error.rs:34:    pub fn serde(msg: impl Into<String>) -> Self {
wavyte-core/src/foundation/error.rs:44:    fn display_prefixes_are_stable() {
wavyte-core/src/foundation/error.rs:68:    fn other_preserves_source() {
wavyte-core/src/effects/transitions.rs:7:pub enum WipeDir {
wavyte-core/src/effects/transitions.rs:15:pub enum TransitionKind {
wavyte-core/src/effects/transitions.rs:20:pub fn parse_transition_kind_params(
wavyte-core/src/effects/transitions.rs:81:pub fn parse_transition(spec: &TransitionSpec) -> WavyteResult<TransitionKind> {
wavyte-core/src/effects/transitions.rs:91:    fn wipe_dir_parses_aliases() {
wavyte-core/src/effects/transitions.rs:108:    fn wipe_soft_edge_is_clamped() {
wavyte-core/src/effects/composite.rs:5:pub type PremulRgba8 = [u8; 4];
wavyte-core/src/effects/composite.rs:7:pub fn over(dst: PremulRgba8, src: PremulRgba8, opacity: f32) -> PremulRgba8 {
wavyte-core/src/effects/composite.rs:32:pub fn crossfade(a: PremulRgba8, b: PremulRgba8, t: f32) -> PremulRgba8 {
wavyte-core/src/effects/composite.rs:46:pub fn over_in_place(dst: &mut [u8], src: &[u8], opacity: f32) -> WavyteResult<()> {
wavyte-core/src/effects/composite.rs:59:pub fn crossfade_over_in_place(dst: &mut [u8], a: &[u8], b: &[u8], t: f32) -> WavyteResult<()> {
wavyte-core/src/effects/composite.rs:77:pub fn wipe_over_in_place(
wavyte-core/src/effects/composite.rs:143:pub struct WipeParams {
wavyte-core/src/effects/composite.rs:151:fn mul_div255(x: u16, y: u16) -> u8 {
wavyte-core/src/effects/composite.rs:155:fn add_sat_u8(a: u8, b: u8) -> u8 {
wavyte-core/src/effects/composite.rs:159:fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
wavyte-core/src/effects/composite.rs:175:    fn over_opacity_0_is_noop() {
wavyte-core/src/effects/composite.rs:182:    fn over_src_alpha_0_is_noop() {
wavyte-core/src/effects/composite.rs:189:    fn over_src_opaque_replaces_dst() {
wavyte-core/src/effects/composite.rs:196:    fn over_dst_transparent_returns_scaled_src() {
wavyte-core/src/effects/composite.rs:203:    fn crossfade_t_0_is_a_and_t_1_is_b() {
wavyte-core/src/effects/composite.rs:211:    fn wipe_ltr_endpoints_match_a_and_b() {
wavyte-core/src/effects/composite.rs:253:    fn wipe_ltr_midpoint_splits_image() {
wavyte-core/src/effects/composite.rs:280:    fn wipe_soft_edge_blends_near_boundary() {
wavyte-core/src/effects/composite.rs:310:    fn wipe_negative_soft_edge_is_treated_as_zero() {
wavyte-core/src/effects/fx.rs:8:pub enum Effect {
wavyte-core/src/effects/fx.rs:15:pub struct InlineFx {
wavyte-core/src/effects/fx.rs:20:impl Default for InlineFx {
wavyte-core/src/effects/fx.rs:21:    fn default() -> Self {
wavyte-core/src/effects/fx.rs:30:pub enum PassFx {
wavyte-core/src/effects/fx.rs:35:pub struct FxPipeline {
wavyte-core/src/effects/fx.rs:40:pub fn parse_effect(inst: &EffectInstance) -> WavyteResult<Effect> {
wavyte-core/src/effects/fx.rs:88:pub fn normalize_effects(effects: &[Effect]) -> FxPipeline {
wavyte-core/src/effects/fx.rs:116:fn get_u32(obj: &serde_json::Value, key: &str) -> WavyteResult<u32> {
wavyte-core/src/effects/fx.rs:131:fn get_f32(obj: &serde_json::Value, key: &str) -> WavyteResult<f32> {
wavyte-core/src/effects/fx.rs:151:fn parse_affine(params: &serde_json::Value) -> WavyteResult<Affine> {
wavyte-core/src/effects/fx.rs:238:    fn inst(kind: &str, params: serde_json::Value) -> EffectInstance {
wavyte-core/src/effects/fx.rs:246:    fn parse_opacity_mul() {
wavyte-core/src/effects/fx.rs:252:    fn normalize_folds_opacity_and_drops_noop_blur() {
wavyte-core/src/effects/blur.rs:3:pub fn blur_rgba8_premul(
wavyte-core/src/effects/blur.rs:32:fn gaussian_kernel_q16(radius: u32, sigma: f32) -> WavyteResult<Vec<u32>> {
wavyte-core/src/effects/blur.rs:75:fn horizontal_pass(src: &[u8], dst: &mut [u8], width: u32, height: u32, k: &[u32]) {
wavyte-core/src/effects/blur.rs:97:fn vertical_pass(src: &[u8], dst: &mut [u8], width: u32, height: u32, k: &[u32]) {
wavyte-core/src/effects/blur.rs:120:fn q16_to_u8(acc: u64) -> u8 {
wavyte-core/src/effects/blur.rs:130:    fn blur_radius_0_is_identity() {
wavyte-core/src/effects/blur.rs:137:    fn blur_constant_image_is_identity() {
wavyte-core/src/effects/blur.rs:146:    fn blur_spreads_energy_from_single_pixel() {
```
