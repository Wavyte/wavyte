# v0.2 Shortlisted Fixes (High Alpha)

Method: I read `REVIEW_1.md` and `REVIEW_2.md`, then verified each candidate directly in `src/` before shortlisting. The list below is intentionally biased toward fixes with high impact and low API churn.

## Shortlist (ranked)

1. **Remove per-frame composition validation from hot render loops**
- Evidence: `src/composition/eval.rs:55` calls `comp.validate()?` inside `eval_frame_with_layout`, and this is invoked per frame from `src/render/pipeline.rs:96` and `src/render/pipeline.rs:343`.
- Why high alpha: avoids repeated full-tree validation work across every frame; direct CPU win with no output behavior change for valid compositions.
- Scope that avoids cornering us: keep validation at API boundaries (`render_*`) and add an internal unchecked eval path for hot loops.

2. **Fix surface lifetime/allocation strategy in CPU backend (reuse surfaces)**
- Evidence: `src/render/cpu.rs:92-110` clears all surfaces and reallocates `Pixmap` for each declared surface every plan execution.
- Why high alpha: this is major CPU + allocator + memory-bandwidth waste per frame.
- Scope that avoids cornering us: change `ensure_surface` to allocate only on miss/size change; keep existing pass model and IR.

3. **Keep final surface alive across readback (avoid churn)**
- Evidence: `src/render/cpu.rs:287` removes the readback surface from the map, forcing re-creation next frame.
- Why high alpha: complements item #2; prevents avoidable lifecycle churn of large frame buffers.
- Scope that avoids cornering us: non-breaking internal change (`get`/copy readback instead of `remove`).

4. **Replace per-frame ffmpeg process spawn for video frame decode**
- Evidence: `src/assets/media.rs:145-158` spawns `ffmpeg` in `decode_video_frame_rgba8` for each frame miss; `src/render/cpu.rs:45-53` calls this in the render path.
- Why high alpha: this is a structural perf bottleneck for video-heavy timelines.
- Scope that avoids cornering us: staged approach (chunk-batch decode or persistent per-asset decoder) without changing public composition schema.

5. **Improve video decoder cache policy (timestamp key + capacity)**
- Evidence: `src/render/cpu.rs:41` fixed capacity `8`; `src/render/cpu.rs:46` cache key is rounded milliseconds.
- Why high alpha: current policy thrashes on common playback patterns and amplifies the cost in item #4.
- Scope that avoids cornering us: make cache size/keying strategy configurable/internally tunable first; no API break required.

6. **Upgrade audio resampling from nearest-floor to linear interpolation**
- Evidence: `src/audio/mix.rs:119` uses `floor()` source frame selection in `mix_manifest`.
- Why high alpha: improves output quality (less aliasing/clicking) and correctness for non-1.0 playback rates.
- Scope that avoids cornering us: confined to mixer internals; can preserve existing manifest format and tests with additive coverage.

7. **Parse and normalize effects/transitions once, not per frame in compile**
- Evidence: effects are cloned as raw JSON in eval (`src/composition/eval.rs:140`) and parsed during compile per node per frame (`src/render/compile.rs:163-170`); transition kind parsing also occurs in compile per pair (`src/render/compile.rs:274-276`).
- Why high alpha: removes repeated JSON parsing/allocation from hot loops and reduces runtime failure surface.
- Scope that avoids cornering us: keep external JSON format unchanged; add typed runtime caches in validated/prepared state.

8. **Avoid frame cloning in static-frame-elision reconstruction path**
- Evidence: `src/render/pipeline.rs:401-410` clones `FrameRGBA` when multiple timeline frames map to one unique rendered frame.
- Why high alpha: large RAM and memcpy overhead for high-resolution static/near-static sequences.
- Scope that avoids cornering us: preserve public `render_frames*` behavior, but for MP4 path stream by index-map without cloning all duplicates.

9. **Harden ffmpeg pipe handling to avoid potential stdin/stderr deadlock class**
- Evidence: encoder writes synchronously to stdin (`src/render/encode_ffmpeg.rs:252`) while stderr is piped and only collected at `finish` (`src/render/encode_ffmpeg.rs:263-269`).
- Why high alpha: robustness under failure/slow encode scenarios; better fault containment.
- Scope that avoids cornering us: add async stderr drain thread (or merge to inherited/null with structured logs), keep current encoder API.

10. **Consolidate duplicated low-level helpers (`Fnv1a64`, `mul_div255`)**
- Evidence: duplicate hashers in `src/assets/store.rs:446` and `src/render/fingerprint.rs:125`; duplicate pixel helper in `src/render/composite.rs:150` and `src/render/encode_ffmpeg.rs:329`.
- Why high alpha: cleanliness + consistency + reduced drift risk in correctness-critical math.
- Scope that avoids cornering us: move to shared internal utility module; no external API impact.

## Notes From Verification

- One review claim was **not accurate as written**: encoder stderr is already captured and surfaced on non-zero exit (`src/render/encode_ffmpeg.rs:267-273`).
- The remaining shortlisted items above were confirmed in source and selected for impact/feasibility balance.
