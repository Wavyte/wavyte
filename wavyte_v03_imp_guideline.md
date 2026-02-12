# Wavyte v0.3 — Production-Grade Implementation Guidelines (Performance-First)

This is the “how we build it” companion to your v0.3 proposal: concrete engineering rules, runtime representations, and perf guardrails so the team can start coding the rewrite without discovering late-stage architectural dead-ends.

The pipeline and scope assumptions are exactly the ones in the proposal (JSON → schema/expr → prepared assets → per-frame layout/eval/compile → CPU backend → ffmpeg encode). 
v0.3 is a hard internal rewrite and a foundation release for downstream crates. 

---

## 0) Non-negotiable perf contract (what “scalable engine” means)

### Frame budget targets

* **Preview target:** **< 33ms/frame @ 1080p on 4 vCPU** (30fps real-time preview). 
* v0.3 overhead (tree walk + per-frame layout + expressions + animated params) must stay in the **low single-digit ms** range in typical scenes (your estimate +2–5ms is plausible only if the hot loop is allocation-free and string-free). 

### Hot-loop invariants (MUST)

**Within the per-frame loop (layout + evaluator + compiler + backend passes):**

1. **No parsing** (no JSON, no expression parsing, no font parsing, no SVG parsing).
2. **No string-keyed maps** (no `HashMap<String, _>` / `BTreeMap<String, _>` lookups).
3. **No heap allocations on the critical path** except bounded “first-use” caches (and those must be measurable + amortized).
4. **No per-frame building of dependency graphs** (expression DAG is prebuilt once). 
5. **No per-frame surface malloc/free churn**: all surfaces are borrowed from a pool.
6. **No per-pass full-frame memcopies** unless unavoidable by semantics; always prefer in-place or ping-pong buffers with pooling.

### Optimization levers to bake in early

These are called out in the proposal; the key is: **design the data structures so these levers are cheap to implement**:

* `Anim::Constant` fast path. 
* Expression AST caching (and ideally bytecode) — parse once, eval many. 
* Color-matrix folding (N color effects → 1 pass). 
* Layout caching with incremental updates. 
* Frame fingerprinting for static-frame elision. 

---

## 1) Architecture: freeze the phase boundaries

The proposal’s pipeline is correct; the rewrite succeeds or fails on **what is precomputed vs per-frame**. 

### Phase A — Load / Validate / Normalize (one-time per Composition)

Output: `CompositionIR` (normalized, indexed, interned)

* Validate schema and structural constraints (unique node ids, ranges valid, assets exist, etc.). 
* Deserialize JSON shorthand into canonical structs (Anim shorthand, effect shorthand, transform shorthand). 
* **Intern and index everything**:

  * Node ids → `NodeIdx(u32)`
  * Asset keys → `AssetIdx(u32)`
  * Effect kind strings → `EffectKindId(u16/u32)`
  * Transition kind strings → `TransitionKindId(u16/u32)`
  * Property paths in expressions → `PropertyId(u32)` (not “nodes.foo.opacity” strings in hot loop)

### Phase B — Compile Expressions (one-time per Composition)

Output: `ExprProgram` + dependency topo order

* Parse all expressions to AST (or better: bytecode). 
* Build dependency graph and topo sort; reject cycles at validation time. 

### Phase C — Prepare Assets (one-time per render invocation; cacheable across invocations)

Output: `PreparedAssetStore` (content-addressed)

* Front-load IO + decode into immutable prepared handles. 
* No ffprobe/ffmpeg process spawning in the per-frame loop—ever.

### Phase D — Per-frame loop (must be tight)

Per frame:

1. Layout solve (Taffy) 
2. Evaluator → `EvaluatedGraph` 
3. Compiler → `RenderPlan` 
4. Backend executes plan → `FrameRGBA` 
5. Encoder consumes frames (pipelined)

---

## 2) Core runtime representation (index-first, strings only at the edges)

The v0.3 scene graph is node-id anchored for GUI/debugging and expressions. 
Keep that at the API boundary, but **never use strings in the hot path**.

### 2.1 `CompositionIR`: normalized + indexed

**Rule:** deserialization outputs “nice structs”, then a normalization pass builds an indexed IR.

Minimal shape:

* `nodes: Vec<NodeIR>` (arena)
* `root: NodeIdx`
* `children: Vec<Vec<NodeIdx>>` stored in `NodeIR` as `SmallVec<[NodeIdx; N]>` for typical arities
* `node_id_to_idx: HashMap<InternedStr, NodeIdx>` exists only for:

  * initial validation
  * stitch “patch node by id”
  * debug output

### 2.2 Interning strategy

You want stable ids *and* fast runtime:

* Intern all node ids / asset keys / effect kinds into a string interner (stable across the composition load).
* Store interned ids as small integers in runtime structs.

**Do not** use `Arc<str>` everywhere in hot structs; it bloats cache lines. Prefer `u32` ids, keep interner in the session.

### 2.3 Property addressing (critical for expressions + incremental)

Define a canonical set of property ids:

* `(NodeIdx, PropertyKey)` → `PropertyId`
* `PropertyKey` is a small enum (opacity, transform.translate.x, layout.width, etc.)
* For nested properties (transform.translate.x), encode as:

  * `PropertyKey::TransformTranslateX`, etc.
  * Or `(base_key, lane)`; either way: **no string walking**.

This is the foundation for:

* expression refs (`nodes.X.opacity`) 
* caching dependency topo order 
* diffability + future incremental rendering hooks 

---

## 3) Expression engine: compile to bytecode, evaluate by topo order

The proposal’s semantics are good: limited arithmetic + refs + builtins + time/vars. 
The key is implementing it without turning evaluation into a hashmap/string festival.

### 3.1 Compile step (one-time)

* Parse expression strings into AST once. 
* Immediately lower AST → bytecode:

  * stack-based VM (fast to implement, fast enough)
  * opcodes: `PushConst`, `LoadProp(PropertyId)`, `LoadVar(VarId)`, `LoadTime(Field)`, `CallBuiltin(fn_id, argc)`, arithmetic ops
* Build dependency edges by scanning bytecode for `LoadProp`.

### 3.2 Dependency order

* Topo sort properties. 
* Store `eval_order: Vec<PropertyId>` and `per_property_program: Vec<BytecodeSlice>`.

### 3.3 Per-frame evaluation

* Maintain a dense `values: Vec<ValueSlot>` indexed by `PropertyId`.
* Each frame:

  1. Seed time/vars (constant per render) 
  2. Evaluate in topo order, write to `values[prop]`.

### 3.4 Types (keep it simple + fast)

Do **not** start with dynamic “JSON Value” expression results.

* Define `ValueSlot` as tagged union optimized for the small set you need:

  * `F64`, `Bool`, `Vec2`, `Color`, maybe `String` only if absolutely necessary
* For v0.3, most expressions should resolve to numeric lanes (opacity, transform lanes, etc.). If you later need string expressions for text, implement as a separate slow path (or restrict).

### 3.5 Cycle detection + error messages

On cycle:

* return schema/validation error with full path (node id + property key), since this happens before render. 

---

## 4) Layout: Taffy integration without rebuilding trees every frame

Per proposal: layout is per-frame, but structure caching is expected. 

### 4.1 Cache the Taffy tree

Maintain in `RenderSession`:

* `taffy: Taffy`
* `node_to_taffy: Vec<Option<TaffyNodeId>>` aligned with `NodeIdx`
* `taffy_root: Option<TaffyNodeId>`

**Rule:** only rebuild the Taffy tree if the scene structure changes (node insertion/removal, child list change). For most renders, structure is static.

### 4.2 Intrinsic measurement (asset-driven)

Layout step requires intrinsic sizes. Proposal flow says: measure leaf sizes from prepared assets, inject as constraints. 
Implementation rules:

* Each prepared asset exposes:

  * `intrinsic_size_px(frame_ctx) -> (w,h)` for dynamic assets (text/video)
  * for static assets (image/svg/path/solidrect): constant
* Cache text shaping results keyed by:

  * font id + size + content hash + max_width + alignment + etc.
* If text content is variable-driven (vars) and vars are constant per render, shape once.

### 4.3 Inject layout → transforms

Proposal: inject layout position as translation offsets into the evaluator’s transform stack. 
Concrete approach:

* Store `layout_rect: RectPx` per `NodeIdx` in a dense vec
* Evaluator reads `layout_rect` and applies `translate += (rect.x, rect.y)` before composing `Affine`.

---

## 5) Evaluator: allocation-free DFS producing a diffable `EvaluatedGraph`

Evaluator is a recursive depth-first walk with range checks, transforms, opacity cascade, and Collection modes (Group/Sequence/Stack/Switch), plus CompRef time remapping. 

### 5.1 Don’t store “evaluated nodes” as heap-rich structs

The proposal’s `EvaluatedGraph` layout is correct conceptually (flat leaves + scoped groups). 
Implementation constraints:

* Preallocate `Vec<EvaluatedLeaf>` and `Vec<EvaluatedGroup>` once and reuse by `clear()` each frame.
* Avoid `Vec` inside `EvaluatedLeaf` for group stack; store:

  * `group_depth: u16`
  * `group_stack: SmallVec<[GroupIdx; 4]>` (most nodes won’t be deeply nested)

### 5.2 Time mapping (Sequence/Switch/CompRef)

Implement time mapping without allocating:

* `EvalContext` contains:

  * `global_frame`
  * `local_frame`
  * `local_range`
  * `time_scale` / `offset` if needed

Sequence mode: compute active child index by cumulative durations (precompute prefix sums for children ranges at normalize time).

Switch mode: sample `active: Anim<usize>` once per frame for that node. 

### 5.3 `Anim<T>` sampling rules (perf critical)

The proposal adds `Anim::Constant` and `Reference` compiled from expressions. 
Guidelines:

* `Anim::Constant`: return by copy, zero branches.
* `Anim::Keyframes`: store keyframe times in a dense vec; sampling does:

  * monotonic cursor optimization (per anim instance keep last index if frame increases)
  * avoid binary search in steady playback
* `Anim::Reference(PropertyRef)`: resolve through `values[PropertyId]` (already computed by topo order).

### 5.4 Resolve effects during eval into a compact representation

Proposal: evaluator outputs fully-resolved static params so compiler never sees `Anim<T>`. 
Concrete requirement:

* **Resolved effect params must not be stored in `BTreeMap<String, _>` in the hot path.**
* Instead:

  * During load/normalize: convert each effect instance into:

    * `EffectKindId`
    * `params: SmallVec<[ParamBinding; 8]>`
  * `ParamBinding` is `(ParamId, AnimParam)` where `ParamId` is an interned small int resolved from the effect’s schema once.
* During eval: produce `ResolvedEffect { kind: EffectKindId, params: SmallVec<[ResolvedParam; 8]> }` (aligned arrays, not maps).

This is the single biggest “don’t regress to v0.2 style dynamic maps” rule.

### 5.5 Group scoping: produce minimal group ops

Group nodes may impose mask/effects over a contiguous leaf range. 
Implementation detail:

* When entering a group that needs isolation (mask or group-level pass effects), record:

  * `start_leaf_idx`
* After processing children, record:

  * `end_leaf_idx`
  * group op with that leaf span

---

## 6) Compiler: build a render-plan DAG, fuse passes, minimize surfaces

Proposal: compiler converts `EvaluatedGraph` → `RenderPlan` with surfaces, passes, composite ops, mask ops, transitions, etc. 

### 6.1 The plan must be a DAG with explicit dependencies

Avoid “execute passes in a vec” unless dependencies are trivial. You will need a DAG as soon as:

* group isolation surfaces exist
* mask surfaces exist
* multiple passes feed into composites

Concrete representation:

* `SurfaceId(u32)`
* `OpId(u32)`
* `RenderPlan { surfaces: Vec<SurfaceDesc>, ops: Vec<Op>, roots: Vec<SurfaceId> }`
* Each `Op` declares input surfaces and output surface.

This enables:

* parallel scheduling of independent ops
* surface lifetime tracking for pooling
* future incremental invalidation at op granularity

### 6.2 Surface allocation strategy (MUST be pooling-friendly)

Compiler must compute:

* required surface descs (w,h,format)
* lifetimes (first use → last use) per surface id

Then the backend requests buffers from a `SurfacePool` using these descs. No malloc churn.

### 6.3 Pass fusion rules (cheap wins)

From proposal:

* fold color-matrix effects into one `PassFx::ColorMatrix` 
* masks compile into mask surfaces + apply ops 

Additional fusion rules to implement in v0.3 baseline:

* **Inline effects** (opacity_mul, transform_post) should be fused into leaf draw state, not passes.
* Consecutive affine-only transforms should multiply into one affine.
* Consecutive opacity multipliers should multiply into one scalar.
* If a pass chain is identity at a frame (e.g., blur radius 0, color matrix = identity), compiler should drop it.

### 6.4 Group isolation heuristic

Render group into offscreen surface **only if required**:

* group has mask, or
* group has pass effects, or
* group participates in certain transitions

Otherwise, directly composite children into parent.

### 6.5 Deterministic plan ordering

To keep fingerprinting and debugging stable:

* ops should be emitted in deterministic order (DFS leaf order + stable group order)
* surface ids deterministic across runs given same composition + frame

This matters for:

* frame fingerprint stability 
* future “diffable evaluated graph” and incremental scheduling hooks 

---

## 7) Backend (CPU): execution model, memory model, and per-pass rules

Proposal calls out CPU backend (`vello_cpu`) producing `FrameRGBA`. 
You can stay CPU-only while still being “engine-grade” if the execution model is disciplined.

### 7.1 SurfacePool (critical)

Implement a `SurfacePool` that:

* buckets allocations by `(w,h,format)`
* returns `SurfaceBuf { ptr, len, stride, generation }`
* supports `clear_to(color)` optimized (SIMD memset where possible)

**Hard requirement:** compiler must request surfaces *by desc*; backend never decides sizes ad-hoc.

### 7.2 Scheduling strategy (v0.3 baseline)

* Build op dependency counts; execute ready ops in parallel using a work-stealing pool.
* Do **not** parallelize by “frames” only; for preview you’ll often render a single frame repeatedly. Op-parallelism helps.

### 7.3 Pixel format rules

Standardize:

* internal surfaces: `RGBA8 premultiplied` (fast compositing)
* only flatten/unpremultiply at the encode boundary (if needed)

### 7.4 Pass implementation guidelines (blur, masks, color matrices, shadows)

* **Blur:** implement separable Gaussian (horizontal + vertical) and reuse scratch buffers from pool.
* **Mask apply:** treat alpha/luma/stencil as different kernels; avoid branches per pixel by selecting a function pointer per mode once. 
* **Color matrix:** operate in premul space carefully; prefer converting to linear float only if required by correctness (most “creative” color ops can be done in 8-bit with LUTs later, but baseline can be f32).
* **Shadows:** multi-pass but share scratch; do not allocate per shadow instance.

### 7.5 Compositing / blend modes

Blend modes list is finite and matches a closed enum. 
Implementation rules:

* all blend mode functions are:

  * branch-free inside the pixel loop
  * chosen once per op (function pointer / match outside loop)
* use SIMD when stable (later), but baseline must be cache-friendly and avoid conversions.

---

## 8) Encoding: pipeline it, don’t serialize it

Proposal ends with ffmpeg encode. 
To be “scalable engine”, v0.3 must **stream** frames to encoder; never accumulate `Vec<FrameRGBA>` for long ranges.

### 8.1 Introduce `FrameSink`

API sketch (conceptual):

* `render_range(session, range, sink: &mut dyn FrameSink)`
* sinks:

  * `FfmpegSink` (mp4)
  * `PngSequenceSink`
  * `InMemorySink` (tests only)

### 8.2 Threading model

At minimum:

* Render threads produce frames
* Encoder thread consumes frames via bounded channel
* Backpressure is applied via the channel bound (not via heap blowup)

### 8.3 Flattening/alpha handling

If encoder wants opaque RGBA:

* flatten premul RGBA against a background once per frame (current approach in v0.2 is correct semantically but is a known hotspot).
  Guidelines:
* Implement a SIMD-accelerated flatten path early (it’s extremely measurable).
* If `FrameRGBA` is guaranteed opaque for many scenes (common), skip flatten entirely.

---

## 9) Fingerprinting + future incremental foundation (must be baked into data shapes now)

Proposal explicitly calls out:

* expanded fingerprinting including groups/effects/expressions 
* incremental hooks: cached expression DAG, incremental layout, prepared assets content-addressed, diffable evaluated graph 
* incremental pipeline execution itself deferred (fine), but *foundation must exist*. 

### 9.1 Define “stable hash” once

Pick one deterministic hash (64/128-bit) and use it everywhere:

* `NodeStateHash` (resolved draw-relevant properties)
* `GroupStateHash`
* `OpHash` in compiler
* `FrameFingerprint`

**Rules:**

* never hash floats by formatting strings; use bit patterns
* include effect kind ids + param ids + param values (post-resolve)
* include mask sources and modes 

### 9.2 Diffable evaluated graph (v0.3 data layout must enable it)

Even if you don’t implement incremental rendering in v0.3, structure the evaluator output so you can later:

* compare leaf hashes across frames
* map leaves to surfaces deterministically
* skip re-render of identical surfaces

Concrete: `EvaluatedLeaf` should include:

* `leaf_hash: u64/128`
* `content_hash: u64/128` (asset + sampled source time + clip range)
* `effect_hash: u64/128`

---

## 10) Concrete “don’t shoot yourself” rules by module

The proposal’s module structure is already reasonable. 
These are the implementation constraints per module that prevent perf regressions.

### `schema/`

* Validation must return structured errors with node id + path.
* Shorthand deserializers must produce canonical forms (no “maybe constant” ambiguity).

### `expression/`

* Parser runs once.
* Runtime eval uses ids, not strings. 

### `scene/`

* Keep serialization structs separate from runtime IR.
* Runtime IR uses `NodeIdx`, `AssetIdx`, `EffectKindId`.

### `assets/`

* Prepared assets are immutable and reference-counted.
* Content-addressing keys must include *all* render-relevant params (font, size, max width, etc.). 

### `layout/`

* Cache Taffy nodes; update styles, don’t rebuild.
* Intrinsic measurement caches are mandatory.

### `eval/`

* Preallocate output vectors.
* Keep context stack small and POD-heavy (affine + opacity + time mapping).
* Resolve effects into compact arrays (no maps).

### `compile/`

* Emit DAG ops with explicit deps.
* Compute lifetimes for pooling.
* Implement fusion rules (color-matrix folding is baseline). 

### `render/`

* Centralize buffer pooling.
* Ensure pass kernels don’t allocate.
* Keep per-pixel loops branch-light.

### `encode/`

* Stream frames. Never keep all frames in memory.

---

## 11) Implementation gates (tests/benchmarks that must pass before moving on)

Use the proposal’s phased plan, but add **perf gates** to avoid “works but slow” surprises. 

### Gate 1 — “Hot loop alloc-free”

* Instrument global allocator (or use a feature flag) and assert:

  * **0 allocations per frame** for a representative scene after warmup.

### Gate 2 — “No string maps in render path”

* Add a CI lint-like check:

  * forbid `HashMap<String,_>` and `BTreeMap<String,_>` in `eval/`, `compile/`, `render/` modules.

### Gate 3 — “Expression perf”

* Benchmark:

  * N=500 expressions, N=2000 expressions
  * ensure per-frame expression eval stays sub-ms on 4 vCPU (typical).

### Gate 4 — “Surface pool stability”

* Render 300 frames:

  * allocations plateau after warmup (pool reaches steady state)
  * surface reuse rate measurable

### Gate 5 — “Plan determinism”

* Same composition + same frame rendered twice:

  * identical `FrameFingerprint` 
  * identical surface/op counts (debug dump)

---

## 12) My take on v0.3 feasibility (without lowballing)

**v0.3 is absolutely “engine-grade” complexity**, mainly because you’re simultaneously introducing:

* a real scene graph (tree + collection semantics + masks) 
* a reactive dependency system (expressions + topo ordering) 
* per-frame layout (Taffy) 
* fully animatable effect params + registry 
* group isolation with render-plan compilation 

That combo is exactly where performance dies if runtime representation isn’t index-based and pooling-based. If you implement the IR/indexing + expression bytecode + compact resolved params + surface pool **first**, the rest becomes additive. If you don’t, you’ll end up doing a second rewrite inside the rewrite.