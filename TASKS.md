# Wavyte v0.2 Revamp Plan (Commit-by-Commit)

Last updated: 2026-02-06
Branch: `wavyte-v0.2`

## Grounding Notes (Current Code + Upstream Docs)

This plan is grounded in:
- Current code and tests in this branch (`cargo test --all-targets` is green as of 2026-02-06).
- Existing dependency versions in `Cargo.toml`.
- Upstream docs for the crates and APIs this revamp depends on.

Primary references:
- `vello_cpu` 0.0.6 docs/features (`u8_pipeline` default, optional `multithreading`, caveat on diminishing returns at high thread counts): https://docs.rs/vello_cpu
- `resvg` 0.46.0 render API (`resvg::render(tree, transform, pixmap)`): https://docs.rs/resvg/latest/resvg/fn.render.html
- `usvg` options/font hooks (`Options.fontdb`, `Options.font_resolver`, `Tree::fontdb` behavior): https://docs.rs/usvg/latest/usvg/struct.Options.html
- `parley` 0.7 layout model (`FontContext`, `LayoutContext`, `RangedBuilder`): https://docs.rs/parley/latest/parley/
- `image` 0.25.x decode path (`load_from_memory`, `DynamicImage::to_rgba8`): https://docs.rs/image/latest/image/fn.load_from_memory.html
- `rayon` thread-pool and parallel iteration APIs (`ThreadPoolBuilder`, `current_num_threads`): https://docs.rs/rayon/latest/rayon/
- `ffmpeg-next` 8.0.0 status/compatibility note (maintenance mode, FFmpeg compatibility statement): https://docs.rs/crate/ffmpeg-next/latest
- FFmpeg official docs for muxing/inputs/options: https://ffmpeg.org/ffmpeg-doc.html

## v0.2 Architecture Decisions

1. CPU-first and CPU-only in v0.2.
- Remove GPU backend code and `gpu` Cargo feature.
- Keep backend internals optimized around `vello_cpu`.

2. Keep `#![forbid(unsafe_code)]` in this crate.
- No `unsafe` blocks in Wavyte code.
- If media decoding requires FFmpeg bindings, isolate in a safe wrapper module and keep the crate surface safe.

3. Replace mutable `AssetCache` runtime loading with immutable eager `PreparedAssetStore`.
- Single-threaded preparation phase.
- Read-only shared asset store during rendering.

4. Parallelize at inter-frame level (chunked), not pass-level.
- Worker-local renderer state.
- Sequential encoding to preserve strict frame order and predictable memory.

5. Add deterministic static-frame elision.
- Fingerprint evaluated/compiled frame state.
- Render once per unique fingerprint in a chunk; clone bytes for duplicates.

6. Full audio/video support is part of v0.2 scope.
- Video decode for frame rendering.
- Audio manifest + mixdown + mux to output MP4.

7. Layout primitives are included, but as a pre-eval transform-resolve pass.
- Track-level layout modes.
- Baked offsets + composable with user animation.

## Commit Sequence

Quality gate for every commit:
- `cargo fmt --all`
- `cargo clippy --all-targets --all-features`
- `cargo test --all-targets`

When a commit changes media/feature flags, run the relevant subset explicitly and keep default CI path green.

### Phase A: Foundation and Surface Simplification

- [x] `v0.2.0 - establish v0.2 baseline metrics and guardrails`
- Add lightweight render benchmark harness outputs for eval/compile/render/encode timing (reusing existing `bench/`).
- Add a `PERF_BASELINE.md` artifact in repo root.
- Acceptance: repeatable baseline numbers generated on the same machine.

- [x] `v0.2.0 - remove gpu feature and backend selection enum`
- Remove `gpu` feature, `BackendKind`, and `create_backend` branching.
- Keep single constructor path for CPU backend.
- Remove `render_vello.rs` from build graph.
- Acceptance: crate compiles/tests pass without `gpu` feature definitions.

- [x] `v0.2.0 - migrate render API to cpu-only backend contracts`
- Update `render.rs`, `pipeline.rs`, and callers to CPU-only entrypoints.
- Keep backend trait seam only where tests need mocking.
- Acceptance: no runtime backend selection remains.

- [x] `v0.2.0 - align cli examples and benches to cpu-only api`
- Update `src/bin/wavyte.rs`, examples, and `bench/` for removed GPU path.
- Remove/replace GPU flags and help text.
- Acceptance: CLI smoke test passes and examples compile.

- [x] `v0.2.0 - remove gpu-specific tests and add cpu coverage replacements`
- Delete or rewrite GPU-only integration tests (`tests/render_gpu*`, parity tests relying on GPU).
- Add CPU determinism/regression equivalents where needed.
- Acceptance: total test suite remains meaningful and green.

- [x] `v0.2.0 - clean cargo manifests and docs after gpu removal`
- Remove `wgpu`, `vello`, `pollster`, GPU feature wiring from root and bench manifests.
- Update README sections that mention GPU feature flow.
- Acceptance: no stale `gpu` references from root manifests/docs.

### Phase B: Immutable Prepared Asset Store

7. `v0.2.0 - introduce prepared_asset_store core types`
- Add `src/asset_store.rs` with:
  - `PreparedAssetStore`
  - `PreparedAsset::{Image,Svg,Text,Path}`
  - `PreparedPath`
  - `AssetId`, `AssetKey`, normalized-path + hash logic (migrated from `assets.rs`).
- Acceptance: module compiles with unit tests for hashing/path normalization.

8. `v0.2.0 - implement eager prepare pipeline for image svg text path`
- Add `PreparedAssetStore::prepare(comp, root)` single-threaded preload.
- Preserve current SVG font policy and custom font resolver behavior.
- Acceptance: all current asset kinds except video/audio prepare successfully.

9. `v0.2.0 - migrate compile to immutable asset lookup api`
- Change compile inputs from `&mut dyn AssetCache` to `&PreparedAssetStore`.
- Stop per-frame path parsing by using `PreparedPath`.
- Acceptance: compile tests updated and green.

10. `v0.2.0 - migrate cpu renderer and pass executor to prepared store`
- `render_cpu.rs` and `render_passes.rs` read from `PreparedAssetStore` only.
- Remove mutable loading calls from hot path.
- Acceptance: render tests remain deterministic and pass.

11. `v0.2.0 - delete legacy asset_cache trait and fs cache implementation`
- Remove `AssetCache` trait and `FsAssetCache` type.
- Keep any reusable helper code moved under `asset_store.rs` or helper module.
- Acceptance: no references to old cache trait remain.

### Phase C: Parallel Chunked Rendering + Static Frame Elision

12. `v0.2.0 - add rayon and render threading configuration`
- Add `rayon` dependency.
- Add pipeline options for chunk sizing and thread count override.
- Acceptance: defaults preserve current behavior when parallelism is disabled.

13. `v0.2.0 - add frame fingerprint module and deterministic tests`
- Add `src/fingerprint.rs` hashing evaluated frame state (and video timestamp once present).
- Add collision-resistance sanity tests and determinism tests.
- Acceptance: stable fingerprints across repeat runs.

14. `v0.2.0 - implement worker-local cpu backend for parallel chunks`
- Add worker-local state (backend + caches) for rayon workers.
- Ensure no cross-thread mutable sharing in render hot path.
- Acceptance: no `Mutex` contention in frame loop core.

15. `v0.2.0 - implement chunked parallel render then sequential encode loop`
- Render unique frames in parallel per chunk.
- Encode in strict frame order per chunk.
- Keep encoder process persistent across chunks.
- Acceptance: output frame ordering correctness validated.

16. `v0.2.0 - add static frame elision in chunk pipeline`
- Deduplicate by fingerprint within chunk.
- Render first occurrence only, clone pixel buffers for duplicates.
- Acceptance: new stat `frames_elided` and tests proving elision behavior.

17. `v0.2.0 - add sequential-vs-parallel parity regression suite`
- Add integration tests comparing byte output:
  - sequential (1 thread)
  - parallel (N threads)
  - multiple chunk sizes
- Acceptance: byte-identical output for deterministic comps.

### Phase D: Video and Audio Asset Support

18. `v0.2.0 - expand model for video and audio timeline controls`
- Extend `VideoAsset` and `AudioAsset` with trim/rate/volume/fades/mute fields.
- Add validation rules for new fields.
- Acceptance: serde roundtrip + validation tests for new schemas.

19. `v0.2.0 - add media abstraction layer and feature-gated ffmpeg backend`
- Add `src/media.rs` abstraction traits for video/audio decode.
- Add optional backend implementation module (FFmpeg-based).
- Keep non-media builds fail-fast with clear runtime errors when media assets are used.
- Acceptance: crate builds both with and without media feature.

20. `v0.2.0 - implement video metadata and per-worker decoder lifecycle`
- Add `VideoSourceInfo` + decoder factory.
- Add worker-local decoder map keyed by `AssetId`.
- Add frame LRU cache per decoder.
- Acceptance: sequential and parallel video render consistency tests.

21. `v0.2.0 - wire video through eval compile and render drawops`
- Add evaluated source timestamp field for video clips.
- Add `DrawOp::Video` and CPU draw path.
- Acceptance: video clip renders with trim/playback-rate correctness tests.

22. `v0.2.0 - add prepared audio decode and storage in asset store`
- Decode audio into normalized interleaved PCM and store in `PreparedAssetStore`.
- Include extraction of video-embedded audio when clip is not muted.
- Acceptance: decode tests for mono/stereo and sample-rate normalization.

23. `v0.2.0 - implement audio manifest builder from timeline`
- Build static audio segment manifest from composition clips.
- Include fades/volume/trim and clip range mapping using rational FPS conversions.
- Acceptance: manifest correctness tests on overlapping segments.

24. `v0.2.0 - implement audio mixer and temp pcm output`
- Add `src/audio_mix.rs` mixdown to `f32le` stereo temp file.
- Clamp and dither policy (if needed) documented and tested.
- Acceptance: audio mixing tests for overlap/fade boundaries.

25. `v0.2.0 - extend ffmpeg encoder for audio muxing`
- Extend encoder config for optional audio input file.
- Update ffmpeg command assembly for dual-input muxing.
- Acceptance: MP4 with audible mixed audio and sync smoke test.

### Phase E: Layout Primitives

26. `v0.2.0 - add layout model types and serde defaults`
- Add `LayoutMode`, `Edges`, alignment enums on `Track`.
- Keep `Absolute` as default.
- Acceptance: old JSON continues to parse with defaults.

27. `v0.2.0 - implement layout resolver pass using prepared asset dimensions`
- Add `src/layout.rs` pre-eval resolver.
- Support `HStack`, `VStack`, `Grid`, `Center`.
- Use intrinsic dimensions from prepared assets.
- Acceptance: deterministic placement unit tests.

28. `v0.2.0 - integrate layout offsets with animated transforms`
- Resolve composition to layout-adjusted clip transforms/offsets before frame loop.
- Ensure animated user transforms compose correctly with layout offsets.
- Acceptance: layout + animation composition tests.

29. `v0.2.0 - extend dsl with video audio and layout builders`
- Add ergonomic DSL APIs for new asset/layout fields.
- Acceptance: DSL-based integration examples compile and validate.

### Phase F: Hardening, Docs, Release

30. `v0.2.0 - add end-to-end media and layout integration tests`
- Add fixtures generated in-test where practical (small synthetic media).
- Cover determinism, A/V sync tolerance, and static-frame-elision with media present.
- Acceptance: CI-stable integration suite.

31. `v0.2.0 - update public docs and migration notes for v0.2`
- Update README and crate-level docs to reflect new pipeline and APIs.
- Add upgrade notes from v0.1 APIs.
- Acceptance: docs match code paths and examples.

32. `v0.2.0 - finalize release metadata and benchmark report`
- Set crate version to `0.2.0`.
- Capture v0.1 vs v0.2 benchmark deltas using the same harness config.
- Acceptance: tagged release candidate state with reproducible benchmark command lines.

## Test Matrix by Milestone

After Phase A:
- `cargo test --all-targets`
- `cargo run --bin wavyte -- --help`

After Phase B:
- `cargo test --all-targets`
- Focused: asset prep + compile + cpu render tests

After Phase C:
- `cargo test --all-targets`
- Focused: new parallel parity tests (`-- --test-threads=1` and default)

After Phase D:
- `cargo test --all-targets --all-features`
- Feature-gated media tests and ffmpeg availability-aware integration smoke tests

After Phase E/F:
- Full gate:
  - `cargo fmt --all --check`
  - `cargo clippy --all-targets --all-features -D warnings`
  - `cargo test --all-targets --all-features`

## Risks and Mitigations

1. FFmpeg binding/toolchain complexity.
- Mitigation: isolate media backend behind a feature + clear build/runtime diagnostics.
- Keep encode path CLI-based even if decode backend uses bindings.

2. Parallel render memory growth at high resolution.
- Mitigation: chunk-size heuristic based on frame byte size and worker count; expose manual override.

3. Non-determinism in parallel execution.
- Mitigation: deterministic sort/order boundaries and parity tests across chunk sizes/thread counts.

4. API churn blast radius.
- Mitigation: maintain commit-level shippable checkpoints and keep compatibility shims briefly where low-cost.

## Explicit v0.2 Non-Goals

- Live preview/editor runtime.
- Web/WASM target work.
- Advanced blend mode expansion beyond current semantics.
- Encode/render pipeline overlap; keep it simple and deterministic for v0.2.
