Here's my review of the Wavyte v0.2.0 codebase based on the walkthrough:

---

## (a) Critical — must address

| # | Issue | Where | Detail |
|---|-------|-------|--------|
| 1 | **Validation called redundantly on every frame** | `eval.rs` → `eval_frame_with_layout` calls `comp.validate()` | In `render_frames_with_stats` and `render_to_mp4_with_stats`, composition is validated once at the top, then `eval_frame_with_layout` re-validates on **every frame**. For a 1800-frame 60fps render, that's 1800 redundant full-tree validations. Should have an internal `eval_frame_unchecked` path post-initial validation. |
| 2 | **Duplicate `Fnv1a64` implementations** | `assets/store.rs` and `render/fingerprint.rs` | Two independent FNV-1a hashers with identical logic. If one diverges (e.g., a bug fix), asset ID stability or fingerprint correctness silently breaks. Extract to `foundation/`. |
| 3 | **Duplicate `mul_div255` implementations** | `render/composite.rs` (returns `u8`) vs `render/encode_ffmpeg.rs` (returns `u16`) | Same math, different return types, living in separate files. A correctness fix in one won't propagate. Unify into a shared pixel-math helper. |
| 4 | **Audio resampling by nearest-floor** | `audio/mix.rs` → `mix_manifest` | Nearest-neighbor resampling at non-unity playback rates introduces audible aliasing/clicks. For a video tool this is a real quality problem — at minimum linear interpolation is needed. |
| 5 | **`FfmpegEncoder` doesn't capture stderr** | `render/encode_ffmpeg.rs` | `finish()` calls `wait_with_output` but the doc doesn't mention stderr handling for diagnostics. If ffmpeg fails (bad codec, corrupt audio), the user gets a generic "process exited non-zero" with no actionable error message. |

---

## (b) Important — should address

| # | Issue | Where | Detail |
|---|-------|-------|--------|
| 1 | **Video frame decoder LRU is "tiny"** | `render/cpu.rs` → `VideoFrameDecoder` | LRU keyed by rounded milliseconds means cache thrashing on normal playback (each frame is a new ms). The rounding granularity and cache size should be tunable, or at least documented as a known perf cliff. |
| 2 | **One surface per node is over-allocation** | `render/compile.rs` | Every evaluated clip gets its own surface + scene pass, even simple `Over` composites with no effects/transitions. For a 20-clip frame, that's 21 surface allocations. Nodes without effects or transitions could share surfaces or be drawn directly into the composite target. |
| 3 | **`parse_effect` is called twice per node** | `render/compile.rs` → `compile_frame` | Effects are already `resolve_effect`'d during eval, then re-parsed from `ResolvedEffect` during compile. This doubles serde_json param parsing per node per frame. Cache the parsed `Effect` in the evaluated node or pass it through. |
| 4 | **Error strings with stale version refs** | `animation/proc.rs`, `render/fx.rs` | Error messages saying "v0.1" in a v0.2 crate confuse users debugging failures. Trivial string fix but important for support/debugging. |
| 5 | **No backpressure on ffmpeg stdin writes** | `render/encode_ffmpeg.rs` | `stdin.write_all(scratch)` on a process pipe can block if ffmpeg falls behind (e.g., slow x264 preset). No timeout, no non-blocking detection. Could deadlock if ffmpeg stderr buffer fills while stdin blocks. |
| 6 | **Layout resolved per API call, not cached** | `render/pipeline.rs` | Layout offsets are recomputed at the top of `render_frame`, `render_frames_with_stats`, and `render_to_mp4_with_stats`. Since composition is immutable and layout is deterministic, this could be computed once and stored alongside `PreparedAssetStore`. |
| 7 | **Encoding runs single-threaded in caller** | `render/pipeline.rs` | Documented in §5.2 but worth flagging: parallel rendering feeds frames into a sequential encoder. At 4K, ffmpeg stdin write + flatten is the bottleneck. A double-buffer / ring-buffer pattern would allow overlap. |

---

## (c) Good to have — address if time allows

| # | Issue | Where | Detail |
|---|-------|-------|--------|
| 1 | **Blur kernel recomputed per frame** | `render/blur.rs` | `gaussian_kernel_q16` recalculates the kernel for every blur pass. For static sigma, the kernel could be cached on `CpuBackend`. |
| 2 | **`svg_raster_params` caps at 16384 silently** | `assets/svg_raster.rs` | Dimension capping without warning could produce unexpected visual results on extreme zoom. A warning log would help debugging. |
| 3 | **`TempFileGuard` uses `Option<PathBuf>` for drop** | `render/pipeline.rs` | An `Option`-wrapped path with `Drop` is fine but fragile — if the guard is leaked (mem::forget), temp files persist. Consider using `tempfile` crate for OS-level cleanup guarantees. |
| 4 | **`VideoSourceInfo::source_fps` returns 0 on den=0** | `assets/media.rs` | Silent zero return on malformed probe data propagates downstream as division-by-zero risk. Should return `Result` or validate during probe. |
| 5 | **`stagger` sorts input, mutating semantics** | `animation/ops.rs` | `stagger(mut anims)` sorts by offset internally, which means caller ordering is silently discarded. Document or accept pre-sorted input. |
| 6 | **`BlendMode` has only `Normal`** | `composition/model.rs` | The enum exists but only has one variant. Carry forward only if blend modes are planned soon; otherwise it adds unused complexity to the compile/composite path. |
| 7 | **No graceful handling of ffmpeg absence at prepare time** | `render/pipeline.rs` | `is_ffmpeg_on_path` is checked at render time, not at composition prepare time. An early check during `PreparedAssetStore::prepare` (when video/audio assets exist) would fail faster with a better error. |

---

**Summary**: The biggest wins are (a1) removing per-frame validation overhead, (a4) fixing nearest-neighbor audio resampling, and (b2/b3) reducing surface over-allocation and redundant effect parsing — these directly impact render perf and output quality without touching the public API.