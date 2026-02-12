# Wavyte v0.3 - Finalized Design Spec (Implementation-Ready)

> Status: Finalized for implementation
> Date: 2026-02-12
> Scope: Full internal rewrite of v0.2.1 internals. No backward compatibility with v0.2.x model.
> Priority rule: When proposal text and implementation guideline conflict, guideline/performance constraints win.

---

## 1. Decision Principles

1. v0.3 is an engine foundation release, not a feature spike.
2. Runtime representation is index-first and pool-first.
3. Strings exist at API/schema edges only.
4. The per-frame loop must be parsing-free, string-map-free, and allocation-free after warmup.
5. We accept extra implementation effort now to avoid rewrite pressure in v0.4/v0.5.

---

## 2. Perf Contract (Non-Negotiable)

### 2.1 Targets

- Preview target: < 33 ms/frame at 1080p on 4 vCPU.
- Typical composition budget assumption:
  - up to 50 leaf nodes
  - up to 10 active pass effects
  - mixed text/image/svg/video layers

### 2.2 Hot-Loop Invariants

Inside per-frame layout + eval + compile + render:

1. No JSON/expression/font/SVG parsing.
2. No `HashMap<String, _>` or `BTreeMap<String, _>` lookups.
3. No unbounded heap allocation; only bounded first-use cache misses.
4. No per-frame dependency graph rebuild.
5. No per-frame surface malloc/free churn.
6. No avoidable full-frame memcpy chains.

### 2.3 Required Early Levers

- `Anim::Constant` fast path.
- Expression bytecode + topo eval.
- Color-matrix folding.
- Taffy tree caching + incremental style updates.
- Surface lifetime analysis + pooling.
- Stable hash surface for fingerprints and future incremental execution.

---

## 3. Pipeline and Phase Boundaries

The pipeline is fixed:

1. Load, validate, normalize -> `CompositionIR`
2. Compile expressions -> `ExprProgram`
3. Prepare assets -> `PreparedAssetStore`
4. Per-frame loop:
  - layout solve
  - evaluator
  - compiler
  - backend
5. Encode via streaming sink (optional audio mixed outside the per-frame loop)

No stage may re-do work owned by an earlier stage.

---

## 4. Two-Layer Data Model (Boundary vs Runtime)

### 4.1 Boundary Layer (Serde/Schema Model)

Purpose:
- JSON IO
- Human-readable IDs
- shorthand forms
- rich error paths

Allowed to use strings and object maps.

### 4.2 Runtime Layer (`CompositionIR`)

Purpose:
- hot-path execution
- compact deterministic structures

Runtime IDs:

- `NodeIdx(u32)`
- `AssetIdx(u32)`
- `EffectKindId(u16)`
- `TransitionKindId(u16)`
- `ParamId(u16)`
- `PropertyId(u32)`
- `VarId(u16)`

Core shape:

```rust
struct CompositionIR {
    nodes: Vec<NodeIR>,
    root: NodeIdx,
    assets: Vec<AssetIR>,
    variables: Vec<VarSlot>,
    expressions: ExprProgram,
    layout_bindings: LayoutIR,
    registries: RegistryBindings,
}
```

`NodeIR` contains only index-addressed references and POD-heavy fields.

### 4.3 Interning Rules

- Node IDs, asset keys, effect kinds, transition kinds, variable names are interned once at load.
- Interned IDs are used by runtime.
- String tables are retained for diagnostics, patch APIs, and logs only.

### 4.4 Property Addressing

Expressions and incremental hooks use canonical property lanes:

- `PropertyKey::Opacity`
- `PropertyKey::TransformTranslateX`
- `PropertyKey::TransformTranslateY`
- `PropertyKey::LayoutWidth`

Minimum v0.3 canonical lane set (not exhaustive, but must be stable in code):

- `Opacity`
- `TransformTranslateX`, `TransformTranslateY`
- `TransformRotationRad`
- `TransformScaleX`, `TransformScaleY`
- `TransformAnchorX`, `TransformAnchorY`
- `TransformSkewX`, `TransformSkewY`
- `SwitchActiveIndex`

Planned-but-disabled in v0.3 (reserve keys to avoid redesign later):

- `LayoutX`, `LayoutY`, `LayoutWidth`, `LayoutHeight` (see expression restrictions in 7.1)

No runtime string path walking like `"nodes.foo.opacity"`.

---

## 5. Scene Graph and Node Semantics

### 5.1 Node Shape

```rust
struct Node {
    id: String, // boundary-only; normalized into NodeIdx
    kind: NodeKind,
    range: FrameRange,
    transform: Anim<Transform2D>,
    opacity: Anim<f64>,
    blend: BlendMode,
    effects: Vec<EffectInstance>,
    mask: Option<MaskDef>,
    transition_in: Option<TransitionSpec>,
    transition_out: Option<TransitionSpec>,
    layout: Option<LayoutProps>,
}
```

### 5.1.1 Time Model and `range` Semantics (Final)

Wavyte v0.3 uses hierarchical time domains:

- `global_frame`: the absolute frame index in the render request.
- Each node’s `range` is defined **in its parent’s local time domain**.

Definitions:

- Node is visible when `parent_local_frame` is in `[range.start, range.end)`.
- Node local frame is `node_local = parent_local_frame - range.start`.
- Root node’s parent local is `global_frame`.

Validation rules:

- All ranges are `start <= end`, end-exclusive.
- For `Group` and `Stack`, children may have arbitrary ranges (relative to the group/stack local time).
- For `Sequence`, child `range.start` must be `0` and `range.end` defines the child duration in sequence time (see 5.2).

### 5.2 Collection Modes (Final Semantics)

#### `Group`

- Applies parent transform/opacity inheritance to children.
- Defines a group scope for optional mask/group-level effects/group transitions.
- Draw order is DFS child order.
- Does not force offscreen isolation by itself.
- Offscreen isolation occurs only when required (mask, group pass effect, transition requiring isolation).

#### `Stack`

- Same timing behavior as `Group`.
- Draw order is DFS child order (last child visually on top).
- No implicit group mask/effect scope semantics unless mask/effects are explicitly set on this node.

#### `Sequence`

- Default behavior: exactly one active child at a given local frame.
- Child durations are `child.range.end` (since `child.range.start == 0` by rule).
- Prefix durations are precomputed in normalization for O(1)/O(log n) active-child resolution.
- Active child receives remapped local frame `sequence_local - child_prefix_start`.

Transition overlap behavior (required for v0.3):

- If adjacent children `A` (out) and `B` (in) have compatible transitions with duration `d > 0`, then the last `d` frames of `A` and first `d` frames of `B` are evaluated in the same sequence-local frame window.
- In the overlap window, both children are evaluated and compiled into a single transition composite at the parent `Sequence` level (compiler handles pairing; evaluator emits both children’s leaves tagged by their child node identity).
- If only one side specifies a transition, behavior matches v0.2.1 “unpaired” rule: attenuate the transitioning side against the other.

#### `Switch { active: Anim<usize> }`

- `active` is sampled once per frame.
- Value is clamped to valid child index range.
- Only active child is evaluated/rendered.
- Inactive children do not execute leaf/effect sampling for that frame.

### 5.3 `CompRef` Semantics (Final)

- Referenced composition is normalized to its own `CompositionIR`.
- Time remapping is explicit via `CompRefTimeMap` fields (offset, scale, optional clip range).
- ID namespace is isolated per composition.
- Cross-composition expression refs are forbidden in v0.3.
- Parent can pass variables to child via explicit binding map only.

This avoids hidden dependency edges and keeps expression DAG bounded and predictable.

### 5.4 Transition Semantics (Final)

Transitions are a parent-level composition rule, not an intrinsic “internal animation” of a node.

Definitions:

- `transition_in`: applies on the first `d` frames of node local time, where `d = duration_frames`.
- `transition_out`: applies on the last `d` frames of node local time.
- `progress` is `k / d` in `[0, 1]`, and is eased by the transition’s easing curve.

Pairing scope:

- Pairing is only performed between adjacent siblings in the same parent collection’s painter order:
  - `Stack`: adjacent layers in DFS painter order.
  - `Sequence`: adjacent children at boundary overlap windows (see 5.2).
- No pairing is performed across unrelated branches or across `CompRef` boundaries.

Implementation note:

- Compiler consumes evaluated “layer units” (leaf units and isolated group units) and applies the same pairing heuristic as v0.2.1: pair when kind-compatible and progress-aligned, else treat as unpaired attenuation.

### 5.5 Mask Semantics (Final)

Masks are group-scoped clipping/compositing primitives.

Boundary model:

```rust
struct MaskDef {
    source: MaskSource,
    mode: MaskMode,
    inverted: bool,
}

enum MaskSource {
    Node(String),      // node id
    Asset(String),     // asset key
    Shape(ShapeDef),
}

enum MaskMode {
    Alpha,
    Luma,
    Stencil { threshold: f32 }, // 0..1
}

enum ShapeDef {
    Rect { width: f64, height: f64 },
    RoundedRect { width: f64, height: f64, radius: f64 },
    Ellipse { rx: f64, ry: f64 },
    Path { svg_path_d: String },
}
```

Runtime rules:

- Masks apply only to `Group`/`Stack` collection scopes (not to individual leaves unless the leaf is isolated as a unit).
- A masked group is isolated into an offscreen surface, then mask is applied as a pass op.
- Mask source is resolved to either:
  - a rendered surface (node mask), or
  - a generated raster surface (asset/shape mask).
- Mask mode is selected once per pass; the pixel loop has no per-pixel branching on mode.

---

## 6. Schema, Shorthand, and Validation

### 6.1 Versioning

- `version` required and must be `"0.3"`.
- v0.2 migration is out of scope for core runtime.

### 6.2 Shorthand Support

Supported at boundary layer:

- `Anim<T>`: constants, keyframes, procedural, expression string.
- Effect shorthand: `{ "blur": 10 }`, `{ "blur": {"radius": 10} }`.
- Transform shorthand with JSON degrees for rotation.

All shorthand is normalized into canonical structs before IR build.

### 6.3 Validation Responsibilities

Validation must catch before render:

- unique node IDs
- asset references
- range validity
- sequence child duration constraints
- expression cycles
- unknown effect/transition kinds
- unknown/missing effect params
- layout/style constraints

Validation output includes structured path + node ID context.

---

## 7. Expression System (Compiled, Typed, Indexed)

### 7.1 Language Scope

Expression source is string prefixed with `=`.

Supported references:

- `self.*`
- `nodes.<id>.*` (same composition only)
- `vars.<name>`
- `time.frame|fps|progress|duration|seconds`

Property ref surface in v0.3 is intentionally small (to keep `PropertyKey` stable and fast):

- `opacity`
- `transform.translate.x|y`
- `transform.rotation_rad`
- `transform.scale.x|y`
- `transform.anchor.x|y`
- `transform.skew.x|y`
- `switch.active` (for Switch nodes only)

Effect params and arbitrary deep object paths are not addressable by expressions in v0.3.

Explicitly forbidden in v0.3 (to avoid layout cycles and expensive dependency surfaces):

- Reading `layout.*` results in expressions.
- Cross-`CompRef` references.

### 7.2 Compile Model

Load-time only:

1. Parse to AST.
2. Lower to bytecode.
3. Resolve refs to `PropertyId` / `VarId`.
4. Build dependency graph.
5. Topo sort and cycle check.

Runtime model:

```rust
struct ExprProgram {
    eval_order: Vec<PropertyId>,
    programs: Vec<PropertyProgram>,
}
```

Where:

```rust
enum PropertyProgram {
    /// Bytecode expression producing a typed ValueSlot.
    Expr(BytecodeSlice),
    /// Samples a literal Anim field (Constant/Keyframes/Procedural) into a ValueSlot.
    SampleNodeLane { node: NodeIdx, lane: PropertyKey },
}
```

`ExprProgram` is paired with per-property metadata:

- `owner_node: NodeIdx` (the node whose context defines `time.*` for this property)
- `value_type: ValueType` (F64/Bool/Vec2/Color)
- `program: PropertyProgram`

### 7.2.1 Time Context for Expressions

`time.*` fields are evaluated in the context of the property’s `owner_node`:

- `time.frame`: the node-local frame (after range start and any sequence remapping), clamped to `[0, time.duration]`
- `time.duration`: the node duration in frames (`range.end - range.start` in its parent domain)
- `time.progress`: `time.frame / time.duration` clamped to `[0, 1]` (duration 0 -> 0)
- `time.seconds`: derived from node-local frame and composition fps

This allows cross-node references without relying on DFS traversal order.

### 7.2.2 Property Set Minimization

To keep per-frame work bounded, `ExprProgram` must only include:

- Properties that are direct expression outputs (a JSON `=` occurrence), and
- Properties that are referenced by other expressions (dependencies).

All other anim sampling occurs directly in the evaluator and does not go through the property VM.

### 7.3 Bytecode VM

Opcode families:

- constants and loads (`PushConst`, `LoadProp`, `LoadVar`, `LoadTime`)
- arithmetic
- comparisons
- built-in calls

Built-ins are selected by function ID, not string name at runtime.

### 7.4 Value Types

`ValueSlot` is a compact tagged union:

- `F64`
- `Bool`
- `Vec2`
- `Color`

String expression output is not supported in the hot path for v0.3.

### 7.5 Variables

- Variables are constants per render invocation.
- Variable override API exists at render-session boundary.
- Animated external data is provided by precomputed keyframes, not by mutating vars each frame.

---

## 8. Animation System

### 8.1 `Anim<T>` Final Shape

```rust
enum Anim<T> {
    Constant(T),
    Keyframes(Keyframes<T>),
    Procedural(Procedural<T>),
    Reference(PropertyId),
}
```

`Reference` is the compiled form; string expressions do not survive normalization.

### 8.2 Sampling Rules

- `Constant`: immediate return.
- `Keyframes`: binary search baseline; optional monotonic cursor optimization must be implemented as per-worker cache (no shared mutation in `CompositionIR`).
- `Reference`: direct slot read from expression values vector.
- `Procedural`: deterministic by seed and frame.

### 8.3 Interpolation

`InterpMode`:

- `Hold`
- `Linear`
- `CubicBezier { x1, y1, x2, y2 }`
- `Spring { stiffness, damping, mass }`
- named non-bezier eases (`elastic`, `bounce`) as dedicated functions

### 8.4 Spring Solver

Analytical solutions for under/critical/over damped regimes.
No per-frame numerical integration loops in hot path.

---

## 9. Asset System and Text Strategy

### 9.1 Core Asset Set

- `Image`, `Svg`, `Text`, `Path`, `Video`, `Audio`
- generated: `SolidRect`, `Gradient`, `Noise`, `Null`

Asset-to-render behavior:

- `Audio` assets produce no visual draw op but participate in the audio pipeline (15.4).
- `Video` assets produce a visual draw op and may participate in audio if the prepared video contains an audio track.

### 9.2 Prepared Assets

- Content-addressed immutable prepared store.
- Decode/probe/shaping in prepare stage only.
- Keys include all render-relevant parameters.

### 9.2.1 Media Controls (Video/Audio)

v0.3 keeps the v0.2.1 media control surface (names may differ at serde layer, but semantics are the same):

- `trim_start_sec`, `trim_end_sec` (optional end)
- `playback_rate` (> 0)
- `volume` (>= 0)
- `fade_in_sec`, `fade_out_sec` (>= 0)
- `mute` (bool)

Rules:

- All media controls are validated at normalize time.
- Source time mapping is pure and deterministic: node-local seconds -> source seconds after trim/rate.

### 9.3 Color Type

`Color` boundary forms:

- rgba object
- hsla object
- hex string
- rgba array

All normalized to runtime RGBA representation and lane-friendly layout.

### 9.4 Text and Layout Dynamism Policy (Balanced)

Final v0.3 decision:

1. Default text content path is render-constant (literal + `vars.*` resolution at render start).
2. Per-frame text content mutation is out of core v0.3 GA path.
3. Dynamic text animation in v0.3 should be achieved via reveal/mask/transform/opacity effects over pre-shaped text.
4. Architecture keeps an extension point (`DynamicTextProgramId`) so v0.4 can add true per-frame text content without IR redesign.

This keeps quality high for product templates while preserving predictable performance.

---

## 10. Layout Engine (Taffy) Final Design

### 10.1 Runtime Objects

`RenderSession` owns:

- `TaffyTree`
- `node_to_taffy: Vec<Option<TaffyNodeId>>`
- `taffy_root: TaffyNodeId`
- `layout_rects: Vec<RectPx>`

### 10.2 Caching Rules

- Taffy node graph is built once per structure version.
- Structure rebuild only on composition structure mutation.
- Style updates are incremental.
- Layout solve runs per frame only when dirty inputs exist.

Dirty sources:

- animated layout properties
- switch/sequence active child changes affecting measured subtree
- intrinsic size changes (video dimension changes, explicit dynamic sources)

### 10.2.1 Visibility in Layout

Layout must ignore nodes that are not visible for the current frame:

- Nodes out of their `range` are treated as `display: none` for layout.
- Inactive `Switch` children are treated as `display: none` for layout.

This prevents hidden timeline segments from affecting flex/grid layout.

### 10.2.2 Animatable Layout Inputs (v0.3 Scope)

Layout inputs may be animated, but they must remain lane-typed and string-free at runtime.

Boundary `LayoutProps` can contain scalar/vec lanes implemented as:

- literal constants
- keyframes
- procedural
- expression references (compiled to `PropertyId` and read via `Anim::Reference`)

Runtime layout bridge updates Taffy styles by sampling these lanes once per frame for layout-participating nodes.

### 10.5 `LayoutProps` Supported Subset (v0.3)

v0.3 implements a pragmatic subset of Flexbox/Grid that is sufficient for template-style layouts and stable performance.

Enums are static (not animatable) in v0.3:

- `display`: `None | Flex | Grid`
- `direction`: `Row | Column`
- `wrap`: `NoWrap | Wrap`
- `justify_content`, `align_items`, `align_content`
- `position`: `Relative | Absolute`

Numeric lanes may be animated (lane-typed):

- `gap_px: Anim<Vec2>`
- `padding_px: EdgesAnim`
- `margin_px: EdgesAnim`
- `flex_grow: Anim<f64>`
- `flex_shrink: Anim<f64>`
- `size`: width/height as `AnimDimension`
- `min_size`, `max_size` as `AnimDimension`

Dimensions (final v0.3 decision):

```rust
enum AnimDimension {
    Auto,                 // static
    Px(Anim<f64>),        // animatable
    Percent(f64),         // static in v0.3
}
```

Edges:

```rust
struct EdgesAnim {
    top: Anim<f64>,
    right: Anim<f64>,
    bottom: Anim<f64>,
    left: Anim<f64>,
}
```

Rationale:

- Animating percent introduces parent-dependent feedback surfaces and complicates caching/incremental layout.
- Supporting `Px(Anim<f64>)` covers common responsive animations (cards growing, columns sliding) with stable performance.

### 10.3 Intrinsic Measurement

Each prepared asset exposes intrinsic dimensions.
For text, shaped metrics are reused from prepared text payload.

### 10.4 Transform Injection

Layout offset is injected before local transform composition:

`world = parent_world * translate(layout_xy) * local_transform`

---

## 11. Effect and Transition Architecture

### 11.1 Registry Traits (Boundary)

Traits remain for extensibility:

- `EffectDef`
- `TransitionDef`

Runtime registry binding rule:

- At session construction, resolve kind strings to ids and build dense lookup tables:
  - `EffectKindId -> EffectImplRef`
  - `TransitionKindId -> TransitionImplRef`
- Per-frame resolve/compile must be id-indexed, not string-indexed.

Schema binding rule:

- Each effect/transition kind defines a stable `ParamSchema` at registry construction time.
- Normalization resolves `(kind, param_name)` to `ParamId` once; per-frame code never consults param names.

### 11.2 Runtime Binding

At normalization:

- kind strings resolve to small IDs
- each effect instance resolves parameter names to `ParamId`

Runtime effect representation:

```rust
struct EffectBinding {
    kind: EffectKindId,
    params: SmallVec<[ParamBinding; 8]>,
}

struct ParamBinding {
    id: ParamId,
    value: AnimParam,
}
```

`AnimParam` is the runtime-typed animatable parameter container:

```rust
enum AnimParam {
    F64(Anim<f64>),
    Vec2(Anim<Vec2>),
    Color(Anim<Color>),
    Bool(bool),       // static
    String(String),   // static
}
```

Resolved runtime shape:

```rust
struct ResolvedEffect {
    kind: EffectKindId,
    params: SmallVec<[ResolvedParam; 8]>,
}
```

`ResolvedParam` is the compact resolved parameter representation:

```rust
enum ResolvedParam {
    F64(f64),
    Vec2(Vec2),
    Color(Rgba8Premul),
    Bool(bool),
    String(String),
}
```

No string maps in evaluator/compiler hot path.

Transition binding follows the same pattern (ids at runtime, no string maps), but transition params are fixed per transition instance in v0.3 (progress is the only per-frame input).

### 11.3 Core Effects

Core includes:

- inline: opacity_mul, transform_post
- pass: blur, color_matrix family, mask apply, shadow family
- draw/mask ops: clip_rect, clip_path

### 11.4 Color Matrix Folding

Consecutive color operations fold into one matrix in compiler.
Identity matrix passes are dropped.

### 11.5 Transitions

Core transitions:

- crossfade
- wipe
- slide
- zoom
- iris

Compiled transitions emit deterministic render primitives and optional masks.

Direction/shape enums (v0.3 closed set):

```rust
enum WipeDir { LeftToRight, RightToLeft, TopToBottom, BottomToTop }
enum SlideDir { Left, Right, Up, Down }
enum IrisShape { Circle, Rect, Diamond }
```

Runtime transition representation mirrors effect binding rules (ids, no hot-path string maps):

```rust
struct TransitionBinding {
    kind: TransitionKindId,
    duration_frames: u32,
    ease: InterpMode, // or a dedicated transition-ease preset; must be numeric-only
    params: SmallVec<[ResolvedParam; 8]>, // fixed per instance in v0.3
}

struct ResolvedTransition {
    kind: TransitionKindId,
    progress: f32, // eased, in [0, 1]
    params: SmallVec<[ResolvedParam; 8]>,
}
```

In v0.3, transition `params` are treated as static per instance; only `progress` varies per frame.

---

## 12. Evaluator Final Design

### 12.1 Execution Order

Per frame:

1. Compute `NodeTimeCtx` for all nodes (node-local frame, duration, and any sequence remapping).
2. Evaluate property programs in topo order (sampling literal lanes and executing expression bytecode) using `NodeTimeCtx` as `time.*` context.
3. Compute per-node visibility for this frame (range checks + switch active selection), then update layout styles and compute Taffy layout if dirty.
4. DFS scene traversal for visible nodes, maintaining transform/opacity inheritance.
5. Resolve leaf draw state, effects, transitions, and emit `EvaluatedGraph`.
6. Record group scopes that require isolation.

Notes:

- Expression evaluation does not depend on DFS traversal order.
- Expressions may reference nodes that are not visible; values are still well-defined via `NodeTimeCtx`.
- `Switch.active` influences visibility/layout participation, but does not change the definition of node-local time for inactive children.

### 12.2 Context Stack

`EvalContext` is POD-heavy:

- `world_affine`
- `effective_opacity`
- `global_frame`
- `local_frame`
- `time_scale`
- `comp_scope`

`NodeTimeCtx` is computed per frame for all nodes and used by:

- expression evaluation (`time.*`)
- sampling `Anim<T>` for properties that are dependencies of expressions
- visibility decisions (range checks)

Minimum shape:

```rust
struct NodeTimeCtx {
    /// Node-local frame before clamping. May be negative/outside duration.
    local_frame_i64: i64,
    duration_frames: u32,
}
```

Sampling rule:

- When sampling animations for expression dependencies, clamp `local_frame_i64` into `[0, duration_frames.saturating_sub(1)]`.

### 12.3 Output Structures

```rust
struct EvaluatedGraph {
    frame: FrameIndex,
    leaves: Vec<EvaluatedLeaf>,
    groups: Vec<EvaluatedGroup>,
    /// Render units are the atomic compositing layers used by the compiler.
    /// A unit is either a single leaf node or an isolated group surface.
    units: Vec<RenderUnit>,
}
```

Hot fields use IDs and compact arrays; debug strings are side-channel only.

`EvaluatedLeaf` includes:

- `node: NodeIdx`
- `asset: AssetIdx`
- `world_transform`
- `opacity`
- `blend`
- `source_time_s`
- `effects: SmallVec<[ResolvedEffect; 4]>`
- `group_stack: SmallVec<[GroupIdx; 4]>`
- `leaf_hash`
- `content_hash`
- `effect_hash`

`RenderUnit` defines the compiler-facing layering boundary:

```rust
enum RenderUnitKind {
    Leaf(NodeIdx),
    Group(NodeIdx),
}

struct RenderUnit {
    kind: RenderUnitKind,
    leaf_range: Range<usize>, // indices into `leaves`
    opacity: f32,             // unit-level opacity (from group inheritance if applicable)
    blend: BlendMode,         // unit-level blend mode
    transition_in: Option<ResolvedTransition>,
    transition_out: Option<ResolvedTransition>,
    unit_hash: u128,
}
```

Compiler pairs transitions and composites at the `RenderUnit` level, not at raw leaf granularity.

### 12.4 Group Isolation Rule

A group gets an offscreen surface only if any condition is true:

1. group has mask
2. group has pass effects
3. transition requires isolated in/out surface

Otherwise children render directly into parent target.

---

## 13. Compiler Final Design (DAG + Lifetimes)

### 13.1 Plan Representation

```rust
struct RenderPlan {
    surfaces: Vec<SurfaceDesc>,
    ops: Vec<Op>,
    roots: SmallVec<[SurfaceId; 2]>,
}

struct SurfaceDesc {
    width: u32,
    height: u32,
    format: PixelFormat,
}

enum PixelFormat {
    Rgba8Premul,
}

struct Op {
    id: OpId,
    kind: OpKind,
    inputs: SmallVec<[SurfaceId; 4]>,
    output: SurfaceId,
    deps: SmallVec<[OpId; 4]>,
}
```

No implicit ordering dependencies.

Minimum required `OpKind` set (v0.3):

```rust
enum OpKind {
    /// Rasterize/draw a set of leaves into `output`.
    Draw { unit: RenderUnitKind, leaves: Range<usize> },
    /// Apply an offscreen effect pass (blur, color matrix, shadow, mask apply).
    Pass { fx: PassFx },
    /// Composite multiple inputs into output (over + blend modes, paired transitions).
    Composite { ops: SmallVec<[CompositeOp; 8]> },
}
```

`PassFx` and `CompositeOp` are closed enums in v0.3 and must be id-indexed at runtime.

```rust
enum PassFx {
    Blur { radius_px: u32, sigma: f32 },
    ColorMatrix { matrix: [f32; 20] },
    MaskApply { mode: MaskMode, inverted: bool },
    DropShadow { offset: Vec2, blur_radius_px: u32, sigma: f32, color: Rgba8Premul },
}

enum CompositeOp {
    Over { src: SurfaceId, opacity: f32, blend: BlendMode },
    Crossfade { a: SurfaceId, b: SurfaceId, t: f32 },
    Wipe { a: SurfaceId, b: SurfaceId, t: f32, dir: WipeDir, soft_edge: f32 },
    Slide { a: SurfaceId, b: SurfaceId, t: f32, dir: SlideDir, push: bool },
    Zoom { a: SurfaceId, b: SurfaceId, t: f32, origin: Vec2, from_scale: f32 },
    Iris { a: SurfaceId, b: SurfaceId, t: f32, origin: Vec2, shape: IrisShape, soft_edge: f32 },
}
```

### 13.2 Deterministic Emission

- DFS leaf/group order drives op ordering.
- Surface IDs are deterministic for identical input and frame.

### 13.3 Fusion Rules

Must implement in v0.3 baseline:

- inline opacity/affine fusion
- color matrix folding
- identity pass elimination
- no-op mask elimination

### 13.4 Surface Lifetime Analysis

Compiler computes first-use/last-use per surface.
Backend receives lifecycle metadata for pooling.

### 13.5 Transition Pairing and Layering

Compiler builds a deterministic `RenderUnit` list (from `EvaluatedGraph.units`) in painter order and then applies transition pairing:

- For each adjacent pair `(A, B)`:
  - If `A.transition_out` and `B.transition_in` are kind-compatible and their `progress` values are aligned within tolerance, compile a single paired transition composite.
  - Otherwise, compile unpaired attenuation (equivalent to v0.2.1 behavior): apply the transitioning side as an opacity/transform/mask attenuation against the other.

Tolerance and compatibility rules are part of v0.3 determinism and must be fixed constants in code (not tunables).

This mirrors v0.2.1 semantics while preserving group isolation and DAG compilation.

---

## 14. CPU Backend Final Design

### 14.1 Surface Pool

`SurfacePool` keyed by `(w, h, format)`.
Borrow/release by plan execution.
No ad-hoc backend allocation decisions.

Memory safety rule:

- Pool growth must be bounded. Provide `session_opts.max_pool_bytes` (or equivalent), and enforce per-bucket and global caps.
- When releasing a surface, if the relevant bucket is at capacity or the pool is above cap, drop the surface buffer instead of retaining it.

### 14.2 Scheduling

- Op DAG execution uses dependency counts and ready queues.
- Independent ops can run in parallel.
- Op-parallelism is required for single-frame preview responsiveness.

Two orthogonal parallelism layers are required in v0.3:

1. Inter-frame parallelism (range renders): rayon worker pool renders frame chunks in parallel.
2. Intra-frame parallelism (single-frame or heavy graphs): DAG op scheduler parallelizes independent ops.

This preserves v0.2.1 strengths while improving single-frame preview latency.

### 14.2.1 Inter-Frame Parallelism Contract (Rayon, Parity with v0.2.1)

For `render_range` and MP4 rendering:

- Keep chunked frame processing with configurable chunk size and worker count.
- Use rayon as the baseline executor for frame-level worker scheduling.
- Support optional static-frame elision by fingerprinting evaluated frames within chunk windows.
- Preserve deterministic output frame order regardless of worker completion order.
- Use worker-local compile/backend state where needed to avoid shared mutable contention.

Thread budgeting rule:

- Session must avoid oversubscription by splitting configured concurrency across frame workers and op workers.
- Default policy:
  - range renders: prioritize frame-level rayon parallelism
  - single-frame preview: prioritize op-level DAG parallelism

### 14.3 Pixel Rules

- Internal format: premultiplied RGBA8.
- Flatten at encode boundary only.

### 14.4 Kernel Rules

- blur: separable gaussian + pooled scratch buffers
- mask apply: mode-selected kernel outside inner loop
- color matrix: one pass over buffer
- blend mode dispatch chosen once per op

### 14.5 Blend Mode Scope (v0.3 Core)

Core blend modes are fixed and closed in v0.3:

- `Normal`
- `Multiply`
- `Screen`
- `Overlay`
- `Darken`
- `Lighten`
- `ColorDodge`
- `ColorBurn`
- `SoftLight`
- `HardLight`
- `Difference`
- `Exclusion`

Non-separable artistic modes (`Hue`, `Saturation`, `Color`, `Luminosity`) are deferred to std-level composition or future core expansion.

---

## 15. Encoding and Frame Sinks

### 15.1 Sink Interface

```rust
trait FrameSink {
    fn begin(&mut self, cfg: SinkConfig) -> WavyteResult<()>;
    fn push_frame(&mut self, idx: FrameIndex, frame: &FrameRGBA) -> WavyteResult<()>;
    fn end(&mut self) -> WavyteResult<()>;
}
```

Built-ins:

- `FfmpegSink`
- `PngSequenceSink`
- `InMemorySink` (tests)

### 15.2 Threading

- render workers produce frames
- encoder thread consumes from bounded channel
- bounded channel is the backpressure control

Ordering contract:

- Sink receives frames in timeline order.
- If parallel workers finish out-of-order, runtime reorders by frame index before `push_frame`.
- For elided duplicates, runtime reuses previously rendered frame payloads while preserving timeline ordering at sink boundary.

### 15.3 Flattening

- premul flatten fast path
- opaque-frame skip path when alpha is known fully opaque

### 15.4 Audio Pipeline (v0.3)

Audio is mixed outside the per-frame render hot loop.

Rules:

- Internal mix format: interleaved stereo `f32le`, sample rate `48_000`.
- Audio sources come from:
  - `Audio` assets, and
  - `Video` assets that contain an audio track (prepared at asset stage).
- Mixing is done for the requested render range and provided to the encoder as a separate input.

Scene-graph constraints (v0.3):

- `Group`/`Stack` time mapping is supported for audio segment extraction.
- `Sequence` time mapping is supported (segments computed from prefix sums and child durations).
- `Switch` time mapping is supported only when `active` is constant over the render range; otherwise v0.3 returns a validation error for audio mixing (visual rendering is still allowed).

This avoids per-frame audio recomputation while keeping the model extensible for future “audio follows switch” support.

Implementation (v0.3 baseline):

- Build an `AudioManifest` by scanning audio-capable leaf nodes whose `range` intersects the render range (independent of visual evaluation).
- Convert frame ranges to sample ranges via fps rational conversion.
- Mix segments into a single PCM buffer with fades/volume/rate applied.
- Provide to `FfmpegSink` as a temp `.f32le` file (baseline) or as a streaming pipe (future optimization).

`FfmpegSink` must not block the render workers: audio generation happens once before frame production starts.

---

## 16. Stable Hashing and Fingerprints

### 16.1 Single Hash Policy

Use one deterministic hash family everywhere for runtime state hashing.

v0.3 choice:

- XXH3 128-bit with fixed seed and explicit endian handling.

Applied to:

- leaf hashes
- group hashes
- op hashes
- frame fingerprints

### 16.2 Float Hashing Rule

Hash float bit patterns, never formatted strings.

### 16.3 Stable Serialization Rule

Stable hashes must be computed over an explicit byte encoding:

- integers serialized as little-endian bytes
- floats serialized by IEEE-754 bit pattern (little-endian)
- slices serialized in deterministic iteration order (the order already enforced by normalized IR)
- structs serialized field-by-field in a fixed order

Never hash debug strings or map iteration order.

---

## 17. Public API Surfaces (Core)

### 17.1 Construction and Validation

- `validate_schema(json)`
- `Composition::from_json(json, registry)`
- `Composition::to_json()`

### 17.2 Session-Oriented Runtime API

v0.3 core introduces `RenderSession` as first-class runtime object:

```rust
RenderSession::new(comp, registry, assets_root, session_opts) -> WavyteResult<RenderSession>
RenderSession::set_variables(overrides)
RenderSession::render_frame(frame, backend, sink?)
RenderSession::render_range(range, backend, sink)
```

This prevents repeated rebuild of expression/layout/runtime caches.

`session_opts` includes a threading profile equivalent in spirit to v0.2.1 `RenderThreading`:

- enable/disable frame-level parallelism
- rayon worker count
- chunk size
- static-frame elision toggle
- op-level parallel budget

### 17.3 Convenience APIs

`render_frame` and `render_to_mp4` remain as thin wrappers creating a transient session.

### 17.4 Downstream Crate Contract

Downstream crate model is unchanged from proposal intent:

- `wavyte`: core runtime and schema
- `wavyte-std`: higher-level presets/builders and optional extra registry definitions
- `wavyte-py` / `wavyte-ts`: JSON-first consumers over core/std registry bundles
- `wavyte-stitch`: headless service using session APIs and patch-by-node-id boundary tooling

---

## 18. Module Structure (Final)

```text
wavyte/src/
├── lib.rs
├── schema/
│   ├── version.rs
│   ├── validate.rs
│   └── shorthand.rs
├── normalize/
│   ├── ir.rs
│   ├── intern.rs
│   ├── property.rs
│   └── bind_registry.rs
├── expression/
│   ├── parser.rs
│   ├── bytecode.rs
│   ├── resolver.rs
│   └── vm.rs
├── scene/
│   ├── model.rs
│   ├── node.rs
│   ├── mask.rs
│   └── dsl.rs
├── animation/
│   ├── anim.rs
│   ├── interp.rs
│   ├── ease.rs
│   ├── spring.rs
│   ├── proc.rs
│   └── ops.rs
├── effects/
│   ├── registry.rs
│   ├── blur.rs
│   ├── color_matrix.rs
│   ├── shadow.rs
│   ├── composite.rs
│   └── transitions.rs
├── assets/
│   ├── store.rs
│   ├── decode.rs
│   ├── media.rs
│   ├── generated.rs
│   └── color.rs
├── audio/
│   ├── manifest.rs
│   └── mix.rs
├── layout/
│   ├── taffy_bridge.rs
│   └── cache.rs
├── eval/
│   ├── evaluator.rs
│   └── context.rs
├── compile/
│   ├── plan.rs
│   ├── compiler.rs
│   ├── fuse.rs
│   └── fingerprint.rs
├── render/
│   ├── backend.rs
│   ├── cpu.rs
│   ├── scheduler.rs
│   └── surface_pool.rs
├── encode/
│   ├── sink.rs
│   └── ffmpeg.rs
├── session/
│   └── render_session.rs
└── foundation/
    ├── core.rs
    ├── error.rs
    └── math.rs
```

---

## 19. Implementation Plan and Gates

This section is the authority on implementation sequencing. Each phase must land as a compilable, testable state with perf gates enforced, not as a long-lived half-integration branch.

### 19.0 Repo Strategy (How to Rewrite Without Chaos)

- Rewrite in-place but keep phase boundaries explicit by introducing new modules per the module map in section 18.
- Avoid “hybrid” hot paths that partially use v0.2.1 types: do conversion at the boundary only.
- Prefer landing phases behind `v03`-prefixed API entrypoints until Phase 8, then switch `lib.rs` exports over in one controlled step.
- Keep v0.2.1 benchmark harnesses and perf baselines intact until the v0.3 harness is ready for apples-to-apples comparison.

### 19.1 Cross-Cutting Implementation Rules

- No strings in `eval/`, `compile/`, `render/` runtime data structures. IDs only.
- No `serde_json::Value` in any hot-path type.
- No dynamic allocation in the per-frame loop after warmup (Gate 1).
- No frame accumulation for long renders: all range APIs must be sink-based.
- Determinism is required: stable ids, stable op ordering, stable fingerprints.

### 19.2 Feature Flags to Add Early

Recommended v0.3 feature flags:

- `media-ffmpeg`: enable ffprobe/ffmpeg-based media probing/decoding (parity with v0.2.1).
- `alloc-track`: enable allocation counters for Gate 1.
- `trace-perf`: lightweight stage timing instrumentation for benchmarks and CI.

### Phase 1 - Foundation and Normalize IR

Goal:

- Establish the boundary/runtime split and get `CompositionIR` building deterministically.

Deliver (minimum):

- new ids/interning/property addressing
- boundary serde model -> canonical boundary structs -> `CompositionIR`
- compact `Anim<T>` core types

Concrete outputs:

- `normalize/intern.rs`: interner and id allocation (NodeIdx/AssetIdx/etc.)
- `normalize/property.rs`: `PropertyKey` and `(NodeIdx, PropertyKey) -> PropertyId` mapping
- `scene/model.rs`: boundary serde structs for Composition/Node/Mask/Assets/Layout
- `normalize/ir.rs`: runtime structs (`CompositionIR`, `NodeIR`, `AssetIR`, `LayoutIR`)

Tests to add:

- schema acceptance/rejection fixtures (unique ids, missing assets, bad ranges)
- determinism tests: same JSON -> same ids and stable ordering
- shorthand roundtrip: JSON constants -> `Anim::Constant`

Gates:

- schema/normalize tests
- no string IDs in runtime structs

### Phase 2 - Expression Bytecode Engine

Goal:

- Parse once, compile to bytecode, and execute by topo order with typed slots.

Deliver (minimum):

- parser + bytecode + topo resolver + VM
- `PropertyId` bindings and cycle errors

Concrete outputs:

- `expression/parser.rs`: parse `=` expressions into AST
- `expression/bytecode.rs`: lower AST -> bytecode + constant pool
- `expression/resolver.rs`: build dependency graph + topo sort + cycle diagnostics
- `expression/vm.rs`: stack VM evaluating into typed `ValueSlot` vec

Tests to add:

- parser golden tests for precedence/associativity
- cycle detection tests with readable error paths
- type mismatch tests (wrong ValueType for target lane)

Gates:

- correctness corpus
- perf benchmark at 500/2000 expressions (sub-ms typical target)

### Phase 3 - Scene Eval Core

Goal:

- Implement `NodeTimeCtx`, visibility selection, DFS evaluation, and `RenderUnit` emission with compact runtime types.

Deliver (minimum):

- DFS evaluator
- group scope emission
- sequence/switch/compref time mapping
- resolved compact effects

Concrete outputs:

- `eval/context.rs`: `NodeTimeCtx`, `EvalContext`, stack structs
- `eval/evaluator.rs`: per-frame pipeline (time ctx -> properties -> visibility -> DFS -> graph)
- `effects/registry.rs` (or equivalent): id-indexed lookup tables for effect/transition impls

Tests to add:

- time mapping tests for nested Group/Sequence/Switch/CompRef
- transition overlap tests for Sequence boundaries
- “inactive switch child does not emit leaves” tests
- render-unit determinism tests (same frame -> same units order)

Gates:

- no hot-loop string maps in `eval/`
- no per-frame allocations after warmup

### Phase 4 - Layout Bridge

Goal:

- Cached Taffy tree, lane-typed style updates, and deterministic layout injection.

Deliver (minimum):

- cached Taffy tree
- incremental style updates
- intrinsic measurement integration

Concrete outputs:

- `layout/taffy_bridge.rs`: build/update tree and compute layout rects
- `layout/cache.rs`: per-node cached intrinsic sizes and “dirty” tracking

Tests to add:

- flex row/column smoke tests
- grid smoke tests
- visibility/layout rule tests (`display:none` when out of range or inactive)

Gates:

- layout parity tests
- dirty/no-dirty microbench

### Phase 5 - Compiler DAG + Fusion

Goal:

- Convert `EvaluatedGraph` + `RenderUnit`s into a deterministic DAG plan with surface lifetimes and fusion.

Deliver (minimum):

- DAG ops
- deterministic emission
- lifetime analysis
- fusion rules

Concrete outputs:

- `compile/plan.rs`: DAG structs + enums (`OpKind`, `PassFx`, `CompositeOp`)
- `compile/compiler.rs`: unit->ops compilation, group isolation, mask compilation
- `compile/fuse.rs`: inline fusion + identity elimination + matrix folding
- `compile/fingerprint.rs`: stable frame fingerprint based on leaf/group/unit hashes

Tests to add:

- “plan is deterministic” (same frame -> same ops/surfaces)
- “identity pass dropped” (blur radius 0, identity color matrix)
- “matrix folding” correctness tests

Gates:

- plan determinism test
- identity-pass elimination test

### Phase 6 - CPU Backend + Surface Pool

Goal:

- Execute DAG plans efficiently with pooled surfaces and pass kernels.

Deliver (minimum):

- pooled surfaces
- DAG scheduler
- pass kernels

Concrete outputs:

- `render/surface_pool.rs`: bounded pool implementation and stats
- `render/scheduler.rs`: dependency-count scheduler with optional parallel execution
- `render/cpu.rs`: vello_cpu draw + kernels for blur/mask/color-matrix/shadow/composite

Tests to add:

- pool plateau test over 300 frames (no unbounded growth)
- per-pass correctness tests (blur endpoints, mask modes, matrix sanity)

Gates:

- pool plateau over 300 frames
- zero allocator churn in steady-state hot loop

### Phase 7 - Streaming Encode Integration

Goal:

- Replace “accumulate frames” patterns with sink-based streaming and integrate audio.

Deliver (minimum):

- `FrameSink`
- bounded render->encode pipeline

Concrete outputs:

- `encode/sink.rs`: `FrameSink` trait + sinks
- `encode/ffmpeg.rs`: ffmpeg process management and stdin streaming
- `audio/` (module to add under v0.3): audio manifest + mix to f32le for ffmpeg input

Tests to add:

- sink ordering tests under parallel render
- long-range streaming smoke (no frame accumulation)

Gates:

- long-range render without frame accumulation
- ffmpeg sink correctness and stability

### Phase 8 - End-to-End Hardening

Goal:

- Replace public API exports, update CLI/examples/benchmarks, and lock performance.

Deliver (minimum):

- wrapper API parity
- benchmark suite
- docs and examples

Concrete outputs:

- update `wavyte-cli` to load v0.3 JSON and render via `RenderSession`
- update `bench` to target v0.3 session API and produce comparable metrics
- port 3-5 representative v0.2.1 example scenes into v0.3 JSON for regressions

Gates:

- performance target check at 1080p
- fingerprint determinism
- regression suite green
- frame-parallel parity check against v0.2.1 behavior (chunking, optional elision, deterministic ordering)

### 19.9 CI and Perf Gates (Concrete Implementation)

Gate 1 - Hot loop alloc-free:

- Implement `alloc-track` using a global allocator wrapper that counts allocations and bytes.
- Add a benchmark/test that warms up once, renders N frames, and asserts allocation count == 0 per frame.

Gate 2 - No string maps in render path:

- Add a repo check script that fails CI if `HashMap<String` or `BTreeMap<String` appears under `wavyte/src/eval`, `wavyte/src/compile`, `wavyte/src/render`.

Gate 3 - Expression perf:

- Add a microbench generating 500 and 2000 property programs and measuring VM eval time per frame.

Gate 4 - Surface pool stability:

- Add a bench that renders 300 frames and asserts pool allocated bytes plateaus after warmup.

Gate 5 - Plan determinism:

- Add a deterministic plan dump (ops + surfaces + hashes) and assert exact equality across two runs.

### 19.10 Corner-Proofing Checklist (Why This Won’t Force a Rewrite Later)

- Scene semantics are explicit and local-time-based (no hidden global timeline assumptions).
- Expressions compile to ids and typed slots; expanding the language does not change hot-path shapes.
- Layout outputs are injected into transforms; future support for expression-reading layout can be added with explicit cycle rules.
- Compiler emits a DAG with lifetimes; GPU backend and incremental invalidation can be added without redesigning plan shape.
- `RenderUnit` abstraction decouples evaluator leaf granularity from compositor layering; group isolation and future caching live here.
- Stable hashing is specified at byte level; incremental rendering can be layered on without changing data model.

---

## 20. Explicitly Deferred from v0.3 GA

1. Cross-`CompRef` expression references.
2. True per-frame mutable text content in core runtime.
3. Incremental render scheduler (full invalidation skipping at op granularity).

Data shapes are designed so these can be added without structural rewrite.

---

## 21. Success Criteria (Final)

v0.3 is complete when all are true:

1. Scene tree semantics match this spec for Group/Stack/Sequence/Switch/CompRef.
2. Hot loop meets invariant checks (no parsing/string maps/alloc churn).
3. Effects and transitions run through ID-bound compact runtime bindings.
4. Compiler emits deterministic DAG plans with surface lifetimes.
5. Backend executes via pooled surfaces and stable scheduling.
6. Layout bridge is cached/incremental and does not rebuild tree per frame.
7. Expression engine uses bytecode/topo order and passes perf gate.
8. Encoding is sink-based and streaming.
9. Bench target is within preview budget envelope for typical scenes.

---

## 22. External Grounding References

Implementation should stay aligned with the following sources:

- Taffy docs and API surface (`TaffyTree`, compute/update methods): https://docs.rs/taffy
- CSS compositing/blending semantics baseline: https://www.w3.org/TR/compositing-1/
- FFmpeg CLI behavior and streaming usage: https://ffmpeg.org/ffmpeg.html
