Here are the biggest **v0.2.x CPU/RAM perf wins** I see after reading the flattened codebase, with **minimal API disturbance** and “staff engineer” levels of ruthlessness. I’m going to call out *exact hotspots* (file + function) and *what to change*.

---

## 0) Biggest performance killers (in order)

### A. You’re doing **full composition validation per frame**

* **Where:** `composition/eval.rs` → `Evaluator::eval_frame_with_layout` has `comp.validate()?;`
* **Why it’s catastrophic:** `validate()` walks tracks/clips/assets and does non-trivial checks. Doing this at 30–60 fps turns “validation” into your hottest loop.
* **Fix:** validate once *before* rendering starts (or cache a “validated” flag / `OnceCell<Result<()>>` inside `Composition`). Keep the “per-frame validate” only under a debug/strict mode.

### B. You **reallocate all surfaces every frame**

* **Where:** `render/cpu.rs` → `CpuBackend::ensure_surface`:

  ```rust
  if id == SurfaceId(0) { self.surfaces.clear(); }
  ```
* **Why it’s catastrophic:** this drops all `Pixmap` allocations → you allocate + touch megabytes per surface per frame. For 1080p that’s ~8MB per surface, per frame, plus allocator overhead.
* **Fix:** never clear the surface store per frame. Reuse surfaces and only reallocate when (width,height) change. This alone can be a night/day speedup.

### C. Video frames are decoded by **spawning FFmpeg per frame**

* **Where:** `assets/media.rs` → `decode_video_frame_rgba8` spawns `ffmpeg` for *every* frame request.
* **Why it’s catastrophic:** process spawn + demux + seek + decode per frame will dwarf everything else. Your renderer can’t be fast if video decoding is “fork bomb as a feature”.
* **Fix (still CPU):** keep a **persistent decoder per video asset** (one process or library context) and stream frames sequentially. More on this below—this is the #1 win if video is common.

### D. Per-pixel compositing is implemented in the “most expensive possible” style

* **Where:** `render/composite.rs` → `over_in_place`, `crossfade_over_in_place`, `wipe_over_in_place`
* **Why it’s bad:** function calls per pixel, clamps per pixel, arrays constructed per pixel, float math per pixel, repeated indexing arithmetic.
* **Fix:** rewrite these as tight row-wise loops with precomputed constants + integer math, and (optionally) SIMD. Details below.

### E. You copy full frames more than you need to

* **Where:**

  * `render/cpu.rs` → `readback_rgba8`: `to_vec()` clones the whole pixmap every frame.
  * `render/pipeline.rs` → static-frame-elision reassembly: clones `FrameRGBA` for duplicates.
* **Fix:** stream to encoder without cloning frame buffers; avoid materializing duplicate frames at all.

---

## 1) Validation & evaluation: make the per-frame loop small

### 1.1 Stop validating in `eval_frame_with_layout`

* **Where:** `composition/eval.rs`
* **Action:** move `comp.validate()` to the beginning of `render_frames` / `render_to_mp4` once; or cache it.
* **Impact:** huge if comp is non-trivial.

### 1.2 Stop cloning strings and hashing strings in hot paths

* **Where:** `composition/eval.rs`

  * Sorting key: `node.clip_id.clone()` inside `nodes_with_key`
  * `assets.id_for_key(&node.asset)` later in compile
* **Action:** internally switch evaluated nodes from `String` keys → **small numeric IDs**:

  * `ClipId: u32` (index in track/clip arrays is enough)
  * `AssetId` already exists—push it into the evaluated graph so compile doesn’t re-hash strings per node per frame.
* **Impact:** big (less allocation, less hashing, better cache locality).

### 1.3 Precompute per-clip constants once

* **Where:** `composition/eval.rs` → `stable_hash64(comp.seed, &clip.id)` per active clip per frame
* **Action:** compute `clip.seed64` once at load/prepare time; store it in the clip struct or a side table.
* **Impact:** moderate, but cheap to do.

### 1.4 Gate tracing instrumentation

* **Where:** `#[tracing::instrument]` on per-frame functions (`eval_frame`, etc.)
* **Action:** `cfg_attr(feature="trace", tracing::instrument(...))`
* **Impact:** can be surprisingly large under load.

---

## 2) Asset preparation: move expensive conversions out of the frame loop

### 2.1 Pre-parse effects and transitions (eliminate serde_json in the frame loop)

* **Where:**

  * `render/compile.rs` parses effects per node per frame:

    * `parse_effect(e.kind.clone(), e.params.clone())`
  * `render/transitions.rs` parse called per frame via `parse_transition_kind_params(...)`
* **Action:** during composition load/validation:

  * Convert `EffectInstance { kind: String, params: Value }` → typed `EffectKind` once
  * Convert `TransitionSpec` → typed `TransitionKind` once
* **Minimal API change path:** keep the external schema the same; internally store both:

  * raw JSON for debugging + re-serialization
  * parsed typed structs for runtime
* **Impact:** large if effects/transitions are common.

### 2.2 Preconvert vector paths into backend-native format once

* **Where:**

  * `render/compile.rs`: `PreparedAsset::Path(a) => DrawOp::FillPath { path: a.path.clone() ... }`
  * `render/cpu.rs`: `bezpath_to_cpu()` converts path each draw
* **Action:** store a backend-friendly path representation in the prepared asset:

  * e.g. cache a `vello_cpu::kurbo::BezPath` (or an equivalent internal “segments” representation)
* **Impact:** huge in vector-heavy workloads.

### 2.3 Text: cache glyph vectors, not just layout

* **Where:** `render/cpu.rs` DrawOp::Text builds `Vec<vello_cpu::Glyph>` per run per frame.
* **Action options:**

  * Store per-run converted glyph arrays in `PreparedText` (best).
  * Or keep a scratch `Vec<Glyph>` in the backend and reuse capacity (cheap win).
* **Impact:** moderate to large depending on text density.

---

## 3) Rendering & compositing: stop throwing cycles into the void

### 3.1 Reuse surfaces across frames (do not clear surface store)

* **Where:** `render/cpu.rs` `ensure_surface`
* **Action:**

  * Keep surfaces in a `Vec<Option<CpuSurface>>` indexed by `SurfaceId.0` (dense IDs).
  * If size matches: reuse.
  * If size differs: reallocate pixmap once.
* **Impact:** *massive* reduction in allocator + memory bandwidth.

### 3.2 Replace `HashMap<SurfaceId, CpuSurface>` with an indexed vector

* **Where:** `render/cpu.rs` surfaces map + repeated `remove()/insert()`
* **Action:** `Vec<Option<CpuSurface>>` (or `Vec<CpuSurface>` with ensured length).
* **Impact:** big (less hashing, fewer moves). Also simplifies code.

### 3.3 Stop constructing a new Vello CPU RenderContext per pass

* **Where:** `render/cpu.rs` → `exec_scene`:

  ```rust
  let mut ctx = vello_cpu::RenderContext::new(width, height);
  ...
  ctx.render_to_pixmap(&mut surface.pixmap);
  ```
* **Action:** reuse a `RenderContext` per surface size (or per backend) if the library allows. Even a small pool helps.
* **Impact:** moderate to large depending on internal allocations.

### 3.4 Rewrite compositing kernels as tight loops (critical)

* **Where:** `render/composite.rs`
* **What’s wrong today:**

  * `over_in_place` calls `over()` per pixel; `over()` clamps opacity and computes `op` per pixel.
  * `wipe_over_in_place` does float `match` per pixel, calls `smoothstep` per pixel, allocates arrays per pixel.
* **Action:** rewrite:

  * Precompute `op_u16` once outside loops.
  * Iterate row-wise using pointers or `chunks_exact_mut(4)` but without per-pixel function calls.
  * Use integer math for lerp/mul-div-255 (there are exact/near-exact fast formulas without division).
  * For wipe with `soft_edge == 0`: do **span copies** (whole regions are pure A or B).
  * For wipe with soft edge: compute `m` incrementally along axis instead of recomputing `pos` via match.
* **Impact:** large, and scales with resolution.

### 3.5 Clearing pixmaps: replace per-pixel copy with word-wise fill

* **Where:** `render/cpu.rs` → `clear_pixmap`
* **Action:** treat buffer as `&mut [u32]` (when aligned) and `fill(packed_rgba_u32)`.
* **Impact:** moderate (clears are pure memory bandwidth).

---

## 4) Encoding: remove work, then micro-optimize what’s left

### 4.1 Don’t flatten alpha in Rust if you can render onto an opaque background

* **Where:** `render/encode_ffmpeg.rs` does `flatten_to_opaque_rgba8(...)` every frame.
* **Observation:** If final surface starts with an **opaque background** (alpha=255), then premultiplied output ends up alpha=255 everywhere after “over” composition. In that case **premul == straight** and you can pipe RGBA directly without flattening.
* **Action:** for MP4 renders, set `RenderSettings.clear_rgba = Some(opts.bg_rgba)` (opaque) and **skip flatten entirely**.
* **Impact:** big (per-pixel math removed).

### 4.2 If you must flatten, make the kernel fast

* **Where:** `flatten_to_opaque_rgba8` uses `/255` per channel per pixel.
* **Action:**

  * Replace division-based `mul_div255` with a fast exact/near-exact multiply-high trick.
  * Process 4–8 pixels per iteration.
  * Optional: SIMD via portable SIMD (Rust toolchain permitting).
* **Impact:** moderate to big depending on how often flatten is needed.

### 4.3 Avoid producing duplicate `FrameRGBA` buffers when using static-frame elision

* **Where:** `render/pipeline.rs` reassembly does `rendered[uniq].clone()`
* **Action:** don’t rebuild `Vec<FrameRGBA>` at all. Instead:

  * Keep `rendered_unique: Vec<FrameRGBA>`
  * While encoding, iterate original frame indices and re-use the same unique frame buffer.
* **Impact:** huge memory reduction when elision hits (slideshows, static scenes).

### 4.4 Consider feeding `rgb24` to FFmpeg for MP4 (optional)

* **Why:** 25% less pipe bandwidth vs RGBA.
* **Tradeoff:** you must drop alpha (but if you’re already opaque, this is safe).
* **Impact:** moderate; helps if pipe/I/O becomes limiting.

---

## 5) Video decoding: the “make it not slow” plan

Right now, decoding is structurally doomed for performance because it’s “spawn ffmpeg per frame”.

### 5.1 Minimum-change, high-win: per-video persistent ffmpeg process

* Start one ffmpeg process per video asset with something like:

  * `-i input -f rawvideo -pix_fmt rgba -vf fps=OUT_FPS ... pipe:1`
* Then read frames sequentially as the timeline advances.
* Handle seeking/jumps by restarting the process or using a keyframe-aware strategy.

### 5.2 Batch decode only the frames you’ll need

* You already compute a per-frame evaluated graph. You can collect required `(video_asset, source_frame_index)` pairs for a chunk, then decode them in one ffmpeg invocation per asset.
* This still uses the ffmpeg CLI, but amortizes process cost heavily.

### 5.3 Best (bigger change): link against libav* and decode in-process

* Still CPU-only; avoids spawn overhead and gives you real control (threading, caching, seeking).
* This is the direction if Wavyte is serious about video-heavy workloads.

---

## 6) Pipeline-level parallelism: keep cores busy without blowing RAM

### 6.1 Preserve backend caches across chunks

* **Where:** `render/pipeline.rs` creates worker backends inside each `render_chunk_parallel_cpu` call → caches die each chunk.
* **Why it matters:** svg raster cache, image cache, video frame cache—all reset repeatedly.
* **Action:** make worker backends **persistent** across the whole render job:

  * either a fixed worker thread pool with channels
  * or a thread-local backend cache that survives chunk calls
* **Impact:** large if assets repeat across chunks (they usually do).

### 6.2 Overlap rendering and encoding with a bounded channel

* Renderer produces frames → encoder thread writes to ffmpeg stdin.
* With bounded capacity, you get backpressure and controlled memory.
* Impact depends on whether ffmpeg write blocks; often helps.

---

## 7) “Free” wins from build/toolchain settings

Even with zero code changes, you can squeeze more out:

* Release profile:

  * `lto = "fat"` (or thin), `codegen-units = 1`, `panic = "abort"`
* Build for the machine:

  * `RUSTFLAGS="-C target-cpu=native"`
* Allocator:

  * consider `mimalloc` / `jemalloc` if allocator shows up in profiles (very likely today because of per-frame pixmap churn).

---

## Suggested execution order (fastest path to big gains)

1. **Remove per-frame `comp.validate()`** (`composition/eval.rs`)
2. **Stop clearing surfaces; reuse pixmaps** (`render/cpu.rs ensure_surface`)
3. **Fix video decoding strategy** (`assets/media.rs` + backend video path)
4. **Skip alpha flatten by rendering onto opaque bg** (`RenderSettings.clear_rgba` + encoder)
5. **Rewrite compositing kernels** (`render/composite.rs`)
6. **Eliminate per-frame serde_json parsing for effects/transitions** (`render/compile.rs` + `render/transitions.rs`)
7. **Kill duplicate frame clones under elision** (`render/pipeline.rs`)
8. Persist caches across chunks (pipeline/worker lifetime)

That sequence tends to turn “slow architecture” into “actually fast engine” without needing GPU magic.

---