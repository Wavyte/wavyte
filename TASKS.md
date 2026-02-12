# v0.3 Commit-by-Commit Implementation Plan (Release-Only)

This file is the authoritative execution plan for implementing Wavyte v0.3 as specified in `wavyte_v03_proposal_final.md`.

## Global Rules (Non-Negotiable)

### Commit message format

All commits must use:

`v0.3 rewrite - Phase <number> : <TASK_TYPE> : <TASK_BRIEF_DESCRIPTION>`

### Release-only loop

All validation runs use release builds. Do not run debug tests as part of the implementation loop.

Canonical verify command set (run before every commit, unless the commit is docs-only):

```bash
cargo fmt --all --check
cargo check --workspace --all-targets --all-features --release
cargo clippy --workspace --all-targets --all-features --release -- -D warnings
cargo test --workspace --all-features --release
cargo doc --workspace --all-features --no-deps
```

In addition:

- If a commit changes CLI behavior: run `cargo run -p wavyte-cli --release -- --help`.
- If a commit changes encoding: run at least one short end-to-end render smoke (added in Phase 8).

### Docs are required

`wavyte` has `#![deny(missing_docs)]`. Any new `pub` API must have rustdoc in the same commit.

### Perf gates are enforced continuously

Introduce the perf gates early and keep them green:

- hot-loop alloc-free after warmup
- forbid string maps in hot modules
- expression perf microbench
- surface pool plateau test
- plan determinism

## Tooling and Workflow (Commit 0.x)

The repo runs all “CI” locally via scripts in `scripts/` (no GitHub Actions for now).

### Commit checklist (Phase 0)

- [x] Commit 0.1: Add v0.3 authority docs
  - Add `wavyte_v03_proposal_final.md` (authority spec) and `TASKS.md` (this file).
  - Add `wavyte_v03_imp_guideline.md` and `wavyte_v03_proposal.md` for historical reference.
  - Validation: canonical verify command set.

- [x] Commit 0.2: Add release verify script
  - Add `scripts/verify_release.sh` that runs the canonical verify command set (release-only).
  - Add `scripts/` docs in `README.md` for how to use it.
  - Validation: run `bash scripts/verify_release.sh`.

- [x] Commit 0.3: Add local guard for string maps in hot modules
  - Add `scripts/forbid_string_maps.sh` that fails if `HashMap<String` or `BTreeMap<String` appears under `wavyte/src/eval`, `wavyte/src/compile`, `wavyte/src/render`.
  - Call it from `scripts/verify_release.sh`.
  - Validation: run `bash scripts/verify_release.sh`.

- [x] Commit 0.4: Add `alloc-track` feature flag scaffold
  - Add `wavyte` feature `alloc-track` and an allocator counter module behind it.
  - Add a no-op default implementation when the feature is off.
  - Validation: run `bash scripts/verify_release.sh`.

Push policy after each commit:
- `git push origin <branch>`

## Phase 1 - Foundation and Normalize IR (Commits 1.x)

Goal: land the boundary/runtime split and build deterministic `CompositionIR` with id-based references.

- [x] Commit 1.1: Add v0.3 module skeletons and feature gating
- Add new module tree per `wavyte_v03_proposal_final.md` section 18, behind a feature like `v03`.
- Ensure `--all-features` builds with docs/clippy (document all `pub` surfaces).
- Validation: canonical verify command set.

- [x] Commit 1.2: Add v0.3 ID types and interner
- Add `NodeIdx`, `AssetIdx`, `EffectKindId`, `TransitionKindId`, `ParamId`, `PropertyId`, `VarId`.
- Add `normalize/intern.rs` string interner and stable id allocation.
- Validation: canonical verify command set.

- [x] Commit 1.3: Add `PropertyKey` and `(NodeIdx, PropertyKey) -> PropertyId`
- Define the stable v0.3 lane set from `wavyte_v03_proposal_final.md` section 4.4 and 7.1.
- Add deterministic ordering rules for ids and property ids.
- Validation: canonical verify command set.

- [x] Commit 1.4: Add boundary serde model (`scene/model.rs`)
- Implement boundary structs for `Composition`, `Node`, `NodeKind`, `CollectionMode`, `MaskDef`, `AssetDef`, `LayoutProps`.
- Implement shorthand deserialization for `Anim<T>` and transforms/effects.
- Validation: canonical verify command set.

- [x] Commit 1.5: Add schema validation skeleton
- Implement `schema/version.rs` and `schema/validate.rs` with structured errors.
- Enforce: version == 0.3, unique node ids, asset refs, range validity basics.
- Validation: canonical verify command set.

- [x] Commit 1.6: Implement normalize pass to `CompositionIR`
- Add runtime structs: `CompositionIR`, `NodeIR`, `AssetIR`, `LayoutIR`, `RegistryBindings`.
- Normalize: intern strings, build node arena, resolve children, resolve asset indices, precompute `Sequence` prefix sums.
- Ensure no strings are retained in runtime structs used by eval/compile/render (keep interner only in session/debug layer).
- Validation: canonical verify command set.

- [x] Commit 1.7: Implement v0.3 `Color` boundary + runtime representation
- Define `Color` boundary forms and normalize to runtime-friendly representation.
- Add stable conversion to premul RGBA used by render.
- Validation: canonical verify command set.

- [x] Commit 1.8: Implement v0.3 `Anim<T>` core + interpolation presets
- `Anim::Constant`, `Keyframes`, `Procedural`, `Reference(PropertyId)` with full serde.
- `InterpMode` including cubic bezier and named presets.
- Spring solver analytical implementation.
- Validation: canonical verify command set.

## Phase 2 - Expression Bytecode Engine (Commits 2.x)

Goal: compiled, typed, id-addressed expressions with topo evaluation and cycle errors.

- [x] Commit 2.1: Expression parser (AST) + golden tests
- Implement recursive-descent parser with precedence.
- Add unit tests for grammar and errors.
- Validation: canonical verify command set.

- [x] Commit 2.2: Lower AST to bytecode + constant pool
- Implement stack VM bytecode format and opcodes.
- Add unit tests for evaluation of arithmetic and built-ins.
- Validation: canonical verify command set.

Commit 2.3: Resolve refs to `PropertyId` and `VarId`
- Implement binding of `nodes.<id>.<lane>` and `self.<lane>` into `PropertyId`.
- Restrict to the v0.3 property ref surface (no deep paths, no effect params, no layout reads).
- Validation: canonical verify command set.

Commit 2.4: Dependency graph + topo sort + cycle diagnostics
- Build edges by scanning bytecode for `LoadProp`.
- Store `ExprProgram` with `eval_order` and per-property metadata (`owner_node`, `value_type`, `program`).
- Reject cycles with readable error paths.
- Validation: canonical verify command set.

Commit 2.5: Expression perf microbench
- Add microbench for N=500 and N=2000 properties.
- Add a CI gate threshold appropriate for the VPS target.
- Validation: canonical verify command set.

## Phase 3 - Scene Eval Core (Commits 3.x)

Goal: allocation-free steady-state evaluation producing `EvaluatedGraph` + `RenderUnit`s.

Commit 3.1: Define `NodeTimeCtx` computation and rules
- Implement per-frame `NodeTimeCtx` for all nodes (local frame + duration).
- Ensure `Sequence` remapping and clamping semantics match spec.
- Validation: canonical verify command set.

Commit 3.2: Property evaluation runtime
- Implement property program execution in topo order using `NodeTimeCtx` for `time.*`.
- Implement `SampleNodeLane` programs (sampling literal anims for expression dependencies).
- Validation: canonical verify command set.

Commit 3.3: Visibility selection for layout and eval
- Implement per-node visibility flags for current frame:
  - range checks
  - switch active child selection
- Ensure invisible nodes are treated as `display:none` for layout.
- Validation: canonical verify command set.

Commit 3.4: DFS evaluator core and context stack
- Implement inherited transform/opacity stack.
- Emit `EvaluatedLeaf` entries (compact ids, smallvec fields).
- Preallocate and reuse vectors per frame (no allocations after warmup).
- Validation: canonical verify command set.

Commit 3.5: Group isolation tagging + `RenderUnit` emission
- Implement rules:
  - isolate only if mask/pass effects/transition requires isolation
- Build `units: Vec<RenderUnit>` in painter order.
- Validation: canonical verify command set.

Commit 3.6: Transition resolution model
- Implement `TransitionBinding` and `ResolvedTransition`.
- Implement `transition_in/out` progress and easing.
- Implement `Sequence` overlap window evaluation behavior.
- Validation: canonical verify command set.

Commit 3.7: Effect binding and resolved param arrays
- Implement `EffectKindId` binding, `ParamId` mapping, `AnimParam`, `ResolvedParam`.
- Add registry binding rule: kind->impl lookup is id-indexed.
- Validation: canonical verify command set.

Commit 3.8: Hot-loop allocation gate test (alloc-track)
- Add a representative scene fixture and a test that asserts 0 allocations per frame after warmup.
- Validation: canonical verify command set.

## Phase 4 - Layout Bridge (Commits 4.x)

Goal: cached Taffy tree with lane-typed style updates and deterministic rect injection.

Commit 4.1: Add Taffy bridge scaffolding
- Create `layout/taffy_bridge.rs` with session-owned tree and `node_to_taffy` mapping.
- Implement structure build from `CompositionIR` for layout-participating nodes.
- Validation: canonical verify command set.

Commit 4.2: Implement supported `LayoutProps` subset
- Implement static enums and animatable numeric lanes as per spec section 10.5.
- Map to Taffy styles; update styles incrementally per frame.
- Validation: canonical verify command set.

Commit 4.3: Intrinsic measurement integration
- Implement intrinsic size for:
  - image/svg/path/solidrect/gradient/noise/null
  - text (using prepared text metrics, render-constant in v0.3)
  - video (static dimensions)
- Validation: canonical verify command set.

Commit 4.4: Layout caching and dirty rules
- Implement “dirty” sources and skip layout solve when not needed.
- Ensure invisible nodes do not influence layout.
- Validation: canonical verify command set.

Commit 4.5: Layout parity tests
- Add focused tests for flex row/column and simple grid.
- Validation: canonical verify command set.

## Phase 5 - Compiler DAG + Fusion (Commits 5.x)

Goal: deterministic DAG plan, surface lifetimes, fusion and stable fingerprinting.

Commit 5.1: Define `RenderPlan` DAG types
- Implement `SurfaceDesc`, `PixelFormat`, `OpKind`, `PassFx`, `CompositeOp`.
- Ensure closed enums and deterministic serialization for hashing.
- Validation: canonical verify command set.

Commit 5.2: Compile `RenderUnit`s into draw ops and surfaces
- Implement unit isolation surfaces.
- Implement leaf draw op mapping for each asset type.
- Validation: canonical verify command set.

Commit 5.3: Implement mask compilation pipeline
- Compile group masks into mask surfaces and `PassFx::MaskApply` ops.
- Implement Node/Asset/Shape mask sources.
- Validation: canonical verify command set.

Commit 5.4: Implement transition pairing at unit level
- Implement pairing heuristic and fixed tolerance constants.
- Add determinism tests for paired vs unpaired behavior.
- Validation: canonical verify command set.

Commit 5.5: Implement fusion rules
- Inline affine/opacity folding.
- Color matrix folding (single matrix pass).
- Identity elimination (blur radius 0, identity matrix, no-op masks).
- Validation: canonical verify command set.

Commit 5.6: Add plan determinism gate
- Add test that compiles the same frame twice and asserts plan dump equality.
- Validation: canonical verify command set.

Commit 5.7: Implement stable hashing and fingerprinting
- Implement byte-level stable encoding for hashes.
- Produce `FrameFingerprint` for elision and determinism.
- Validation: canonical verify command set.

## Phase 6 - CPU Backend + Surface Pool (Commits 6.x)

Goal: pooled execution of DAG plan with parallel scheduling and correct kernels.

Commit 6.1: Surface pool implementation with caps
- Implement bucketed pool keyed by desc; enforce global and per-bucket caps.
- Expose pool stats for benches/tests.
- Validation: canonical verify command set.

Commit 6.2: DAG scheduler implementation
- Dependency-count ready queue.
- Optional parallel op execution when safe.
- Validation: canonical verify command set.

Commit 6.3: vello_cpu draw implementation for leaf draws
- Implement draw ops for image/svg/text/path/video/solidrect/gradient/noise (as available).
- Keep caches out of hot path except bounded first-use caches.
- Validation: canonical verify command set.

Commit 6.4: Implement blur kernel (pooled scratch)
- Separable gaussian; test endpoints and invariants.
- Validation: canonical verify command set.

Commit 6.5: Implement mask apply kernel
- Alpha/luma/stencil modes, mode-selected function outside pixel loop.
- Validation: canonical verify command set.

Commit 6.6: Implement color matrix kernel
- Single-pass matrix; add correctness tests on small buffers.
- Validation: canonical verify command set.

Commit 6.7: Implement blend/composite ops
- Implement v0.3 closed blend set.
- Ensure branch-free inner loops, dispatch outside loops.
- Validation: canonical verify command set.

Commit 6.8: Surface pool plateau gate
- Render 300 frames and assert pool bytes plateau after warmup.
- Validation: canonical verify command set.

## Phase 7 - Streaming Encode Integration + Audio (Commits 7.x)

Goal: sink-based streaming and audio mixing outside per-frame loop.

Commit 7.1: Add `FrameSink` trait and `InMemorySink`
- Implement sink interface and ordering contract.
- Validation: canonical verify command set.

Commit 7.2: Add `FfmpegSink` (video-only)
- Stream RGBA frames to ffmpeg stdin.
- Implement flattening fast path and opaque skip path.
- Validation: canonical verify command set.

Commit 7.3: Add audio manifest and mixer modules
- Add `audio/manifest.rs` and `audio/mix.rs` per spec 15.4.
- Enforce Switch audio constraint: active must be constant over range (else validation error for audio).
- Validation: canonical verify command set.

Commit 7.4: Integrate audio into `FfmpegSink`
- Provide `.f32le` temp file input (baseline).
- Ensure ffmpeg spawn and teardown are robust.
- Validation: canonical verify command set.

Commit 7.5: Range render streaming implementation
- Implement `RenderSession::render_range` with:
  - chunking
  - rayon frame-level parallelism
  - optional static-frame elision
  - deterministic reordering into sink
- Validation: canonical verify command set.

## Phase 8 - Public API Switch, CLI/Bench, Production Hardening (Commits 8.x)

Goal: flip exports to v0.3, ship production-ready crate, and validate publish dry run.

Commit 8.1: Add v0.3 JSON fixtures and example scenes
- Add a minimal set of reference v0.3 JSON compositions used by tests and CLI smoke.
- Validation: canonical verify command set.

Commit 8.2: Update `wavyte-cli` to v0.3
- CLI loads v0.3 JSON, validates schema, prepares assets, renders frame and MP4.
- Add CLI integration tests (release-only).
- Validation: canonical verify command set + CLI `--help` run.

Commit 8.3: Update `wavyte-bench` for v0.3 session API
- Ensure benchmark measures stage timings relevant to v0.3.
- Add perf baseline capture instructions.
- Validation: canonical verify command set.

Commit 8.4: Remove or quarantine v0.2.1 public exports
- Switch `wavyte/src/lib.rs` exports to v0.3 API surface.
- Keep v0.2.1 code only if needed for historical reference, but do not expose it publicly.
- Validation: canonical verify command set.

Commit 8.5: Hardening pass (docs, errors, determinism)
- Ensure all public APIs have docs.
- Ensure validation errors are structured and actionable.
- Ensure plan determinism and fingerprints are stable.
- Validation: canonical verify command set.

Commit 8.6: Version bump to `0.3.0` and docs update
- Update `wavyte/Cargo.toml` and related version metadata.
- Update `README.md` with v0.3 usage.
- Validation: canonical verify command set.

Commit 8.7: Publish dry run validation (production-ready endpoint)
- Run and record results in the PR description/notes:
  - `cargo publish -p wavyte --dry-run`
  - `cargo package -p wavyte`
- Ensure `cargo doc` output is clean and docs.rs metadata is valid.
- Validation: canonical verify command set + publish dry run commands.

## Definition of Done for the Branch

By the last commit:

- `cargo fmt --all` produces no diff.
- `cargo clippy --workspace --all-targets --all-features --release -- -D warnings` is clean.
- `cargo test --workspace --all-features --release` is green.
- `cargo doc --workspace --all-features --no-deps` is green.
- Perf gates are present and green.
- CLI renders at least one v0.3 fixture end-to-end on release build.
- `cargo publish -p wavyte --dry-run` passes.
