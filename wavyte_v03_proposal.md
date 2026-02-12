# Wavyte v0.3 — System Design Proposal

> **Status**: Proposal  
> **Date**: 2026-02-12  
> **Scope**: Foundation release. Replaces v0.2.1 internals entirely. No backward compatibility.  
> **Goal**: Production-grade core that enables wavyte-std, wavyte-py, wavyte-ts, and wavyte-stitch as downstream consumers.

---

## 1. Architectural Overview

### 1.1 Pipeline (v0.3)

```
                    ┌────────────────────────────────────────────────┐
                    │              Composition (JSON)                │
                    │  version · canvas · duration · fps · seed      │
                    │  assets{} · variables{} · root: Node           │
                    └──────────────────┬─────────────────────────────┘
                                       │
                              ┌────────▼────────┐
                              │   Schema Valid.  │  JSON Schema check
                              └────────┬────────┘
                                       │
                              ┌────────▼────────┐
                              │   Expression     │  Resolve $var refs,
                              │   Resolver       │  evaluate expr()
                              └────────┬────────┘
                                       │
                              ┌────────▼────────┐
                              │   Asset Store    │  IO/decode (front-loaded)
                              │   Prepare        │  PreparedAssetStore
                              └────────┬────────┘
                                       │
                      ┌────────────────▼────────────────┐
                      │         Per-Frame Loop           │
                      │  ┌──────────────────────────┐   │
                      │  │  Layout Solve (Taffy)     │   │  per-frame
                      │  └────────────┬─────────────┘   │
                      │  ┌────────────▼─────────────┐   │
                      │  │  Evaluator (scene tree    │   │  sample Anim<T>,
                      │  │  walk, recursive)         │   │  resolve transforms
                      │  └────────────┬─────────────┘   │
                      │  ┌────────────▼─────────────┐   │
                      │  │  Compiler → RenderPlan    │   │  surfaces, passes
                      │  └────────────┬─────────────┘   │
                      │  ┌────────────▼─────────────┐   │
                      │  │  Backend (CPU) → FrameRGBA│   │  vello_cpu
                      │  └──────────────────────────┘   │
                      └─────────────────────────────────┘
                                       │
                              ┌────────▼────────┐
                              │  Encode (ffmpeg) │
                              └─────────────────┘
```

**Key changes from v0.2.1:**

| Aspect | v0.2.1 | v0.3 |
|---|---|---|
| Composition shape | `tracks[] → clips[]` (flat) | `root: Node` (tree) |
| Layout | static, one-shot solve | per-frame via Taffy |
| Animation values | `Anim<T>` (Keyframes\|Procedural\|Expr) | + `CubicBezier` interp, + `Ref` expressions, shorthand deser |
| Effect params | static `serde_json::Value` | `Anim<T>`-valued params, resolved per-frame |
| Expression language | none | `expr()` in JSON for cross-property refs + math |
| Extensibility | closed enums | trait-based registry (static dispatch) |
| JSON schema | none | versioned, validated, with shorthand sugar |

### 1.2 Crate Topology

```
wavyte/                  ← core engine (this proposal)
wavyte-std/              ← high-level abstractions, effect library
wavyte-py/               ← Python bindings (PyO3), consumes JSON
wavyte-ts/               ← TypeScript bindings (napi-rs/WASM), consumes JSON
wavyte-stitch/           ← headless server binary (HTTP/WS/SSE)
```

All four consumers speak the **same JSON composition format**. wavyte-std additionally has direct Rust API access to core types, bypassing JSON for ergonomics.

---

## 2. Composition Model — Scene Graph

### 2.1 Core Primitives

The scene is a **directed acyclic graph** (DAG) of nodes rooted at a single `Node`. Three fundamental concepts:

```
┌──────────────────────────────────────────────────────┐
│  Composition                                         │
│  ├── version: "0.3"                                  │
│  ├── canvas: { width, height }                       │
│  ├── fps: { num, den }                               │
│  ├── duration: u64 (frames)                          │
│  ├── seed: u64                                       │
│  ├── assets: { key → AssetDef }                      │
│  ├── variables: { name → Value }                     │
│  └── root: Node                                      │
│                                                      │
│  Node (the atomic unit)                              │
│  ├── id: String (stable, unique within composition)  │
│  ├── kind: NodeKind                                  │
│  ├── range: FrameRange (timeline placement)          │
│  ├── transform: Anim<Transform2D>                    │
│  ├── opacity: Anim<f64>                              │
│  ├── blend: BlendMode                                │
│  ├── effects: [EffectInstance]                        │
│  ├── mask: Option<MaskDef>                           │
│  ├── transition_in: Option<TransitionSpec>            │
│  ├── transition_out: Option<TransitionSpec>           │
│  └── layout: Option<LayoutProps>                     │
│                                                      │
│  NodeKind                                            │
│  ├── Leaf { asset: String }                          │
│  ├── Collection { mode: CollectionMode,              │
│  │                children: [Node] }                 │
│  └── CompRef { composition: String | inline Comp }   │
│                                                      │
│  CollectionMode                                      │
│  ├── Group      ← children composited together       │
│  ├── Sequence   ← children play end-to-end           │
│  ├── Stack      ← children overlap, z-ordered        │
│  └── Switch { active: Anim<usize> }                  │
│              ← one child visible at a time            │
└──────────────────────────────────────────────────────┘
```

### 2.2 How Tracks Map to This

v0.2.1 `Track` with `Clip[]` is sugar for:

```json
{
  "id": "track_main",
  "kind": { "Collection": { "mode": "Stack", "children": [ ...clips... ] } },
  "range": [0, 300]
}
```

The `Track` concept is **gone from core**. wavyte-std re-introduces it as a builder helper that emits `Collection(Stack, [...])` nodes.

### 2.3 Node Identity and References

Every node has a stable `id`. This is the anchor for:

- Expression references: `expr("nodes.card_bg.transform.translate.x")`
- GUI manipulation: wavyte-stitch targets nodes by id
- Debugging/tracing: evaluator logs reference node ids

IDs must be unique within a composition (validated at schema level). Nested `CompRef` compositions have their own id namespace.

### 2.4 Masks

```
MaskDef
├── source: MaskSource
│   ├── Node(node_id)    ← another node's rendered output as mask
│   ├── Asset(asset_key) ← asset used directly as mask
│   └── Shape(ShapeDef)  ← inline shape (rect, ellipse, path)
├── mode: MaskMode
│   ├── Alpha            ← use source alpha channel
│   ├── Luma             ← use source luminance
│   └── Stencil          ← binary threshold
└── inverted: bool
```

Masks cascade: a `Collection(Group)` mask clips all children. This is how rounded-rect clipping works — the group carries a `Shape(RoundedRect)` mask.

---

## 3. Asset System

### 3.1 Asset Variants

Assets remain a **closed set in core**, extended by wavyte-std registering additional types. v0.3 core assets:

```
AssetDef
├── Image    { source: String }
├── Svg      { source: String }
├── Text     { text, font_source, size_px, color, max_width_px, ... }
├── Path     { svg_path_d: String }
├── Video    { source, trim_start_sec, trim_end_sec, playback_rate, ... }
├── Audio    { source, trim_start_sec, ... }
│
│  ── NEW in v0.3 ──
├── SolidRect    { width, height, color: Anim<Color>, corner_radius: Anim<f64> }
├── Gradient     { kind: Linear|Radial|Conic, stops: [...], ... }
├── Noise        { kind: Perlin|Simplex|Worley, scale, octaves, seed }
└── Null         {}   ← invisible; used as mask source, group anchor, etc.
```

`SolidRect`, `Gradient`, `Noise` are **generated assets** — no file reference, procedurally rendered. They live as core asset variants because they appear in 2+ high-level std abstractions (backgrounds, shape fills, texture overlays).

### 3.2 Color Type

v0.2.1 uses `[u8; 4]` or `Rgba8Premul` everywhere. v0.3 introduces a proper `Color` type that supports multiple spaces:

```rust
enum Color {
    Rgba { r: f64, g: f64, b: f64, a: f64 },     // 0..1 range
    Hsla { h: f64, s: f64, l: f64, a: f64 },
    Hex(String),                                    // "#rrggbbaa"
}
```

`Color` implements `Lerp` (interpolates in the color space it's defined in; HSLA lerps hue on the short arc). JSON accepts any form; internal evaluation normalizes to RGBA for rendering.

**Shorthand in JSON:**
```json
"color": "#ff0000"           // hex
"color": [1.0, 0.0, 0.0, 1.0]  // rgba array
"color": { "Hsla": { "h": 0, "s": 1, "l": 0.5, "a": 1 } }  // explicit
```

---

## 4. JSON Schema and Expression Language

### 4.1 Schema Versioning

```json
{
  "version": "0.3",
  "canvas": { "width": 1920, "height": 1080 },
  "fps": { "num": 30, "den": 1 },
  "duration": 300,
  ...
}
```

- `version` is **required** and must be `"0.3"`.
- Core exposes `validate_schema(json: &str) -> Result<(), Vec<SchemaError>>` as a standalone function. Bindings call this before submitting.
- From v0.4 onward, a `migrate(json, from_version, to_version)` function will be provided. No migration from v0.2.

### 4.2 Anim Shorthand Deserialization

The single biggest JSON ergonomic win. Every `Anim<T>` field accepts:

| JSON form | Interpretation |
|---|---|
| `1.0` | `Anim::Constant(1.0)` |
| `"#ff0000"` | `Anim::Constant(Color::Hex("#ff0000"))` |
| `[100, 200]` | `Anim::Constant(Vec2(100, 200))` (for Vec2 fields) |
| `{ "keyframes": [...] }` | Full keyframe definition |
| `{ "proc": {...} }` | Procedural source |
| `{ "expr": "..." }` | Expression reference |

Implementation: custom `Deserialize` for `Anim<T>` that first attempts to parse `T` directly (constant), then falls back to the full tagged enum.

### 4.3 Effect/Transform Shorthand

```json
// v0.2.1 — verbose
{ "kind": "blur", "params": { "radius_px": 10 } }

// v0.3 — shorthand
{ "blur": 10 }
{ "blur": { "radius": 10, "sigma": 5.0 } }

// Transforms
"transform": { "translate": [100, 200], "scale": 2.0, "rotate": 45 }
// (rotate in degrees by default in JSON; internally stored as radians)

// Animated shorthand
"transform": {
  "translate": { "keyframes": [
    { "frame": 0,  "value": [0, 0] },
    { "frame": 30, "value": [100, 200], "ease": "out_cubic" }
  ]}
}
```

### 4.4 Expression Language

Position B: JSON values can contain **expressions** that reference other properties and perform arithmetic.

**Syntax**: Expressions are strings prefixed with `=`:

```json
{
  "id": "follower",
  "kind": { "Leaf": { "asset": "circle" } },
  "transform": {
    "translate": ["= nodes.leader.transform.translate.x + 50", "= nodes.leader.transform.translate.y"]
  },
  "opacity": "= clamp(time.progress * 2, 0, 1)"
}
```

**Expression runtime**:

```
Expression Grammar (v0.3 subset):
  expr     → term (('+' | '-') term)*
  term     → factor (('*' | '/') factor)*
  factor   → NUMBER | STRING | ref | call | '(' expr ')'
  ref      → 'nodes.' ID ('.' ID)*          // cross-node property ref
           | 'self.' ID ('.' ID)*            // self-property ref
           | 'vars.' ID                      // composition variable ref
           | 'time.' FIELD                   // time context
  call     → ID '(' expr (',' expr)* ')'    // built-in functions
  
Built-in context:
  time.frame          → current frame (u64)
  time.fps            → fps as f64
  time.progress       → clip_local / clip_duration (0..1)
  time.duration       → clip duration in frames
  time.seconds        → current time in seconds

Built-in functions:
  sin(x), cos(x), abs(x), min(a,b), max(a,b), clamp(x,lo,hi),
  lerp(a,b,t), smoothstep(lo,hi,t), random(seed), floor(x), ceil(x),
  pow(base,exp), sqrt(x), mod(a,b), if(cond, then, else)
```

**Implementation**: A small expression parser and evaluator compiled into core. Not a general-purpose language — intentionally limited to arithmetic + property reads. Evaluation happens **during the Evaluator phase**, after the expression resolver topologically sorts property dependencies.

**Dependency resolution**: Before per-frame evaluation, the expression resolver:

1. Parses all expression strings in the composition into ASTs (cached)
2. Builds a dependency graph: `follower.translate.x` depends on `leader.translate.x`
3. Topologically sorts: evaluate `leader` before `follower`
4. Detects cycles → error at validation time

This is the critical enabler for "attach to", "follow", and responsive behaviors. Without it, the scene graph is just a render tree. With it, it's a live reactive graph.

### 4.5 Variables

Composition-level variables, settable at render time:

```json
{
  "variables": {
    "title_text": "Hello World",
    "brand_color": "#ff5500",
    "show_subtitle": true
  },
  "root": {
    "id": "title",
    "kind": { "Leaf": { "asset": "title_asset" } },
    "opacity": "= if(vars.show_subtitle, 1.0, 0.0)"
  }
}
```

Variables are the primary mechanism for template-based video generation (wavyte-py passes different variables per render). They are **not animated** — they are constants per render invocation. For animated external input, use `Anim::Keyframes` in the variable slot (wavyte-std can provide helpers to inject keyframe data from external sources).

---

## 5. Animation System

### 5.1 Anim\<T\> Redesign

```rust
enum Anim<T> {
    Constant(T),                    // NEW: explicit constant (no keyframe overhead)
    Keyframes(Keyframes<T>),
    Procedural(Procedural<T>),
    Expr(Expr<T>),
    Reference(PropertyRef),         // NEW: "= nodes.x.opacity"
}
```

`Constant` is a first-class variant (not a single-keyframe hack). This matters for:
- JSON shorthand: bare values deser directly to `Constant`
- Performance: skip keyframe binary search for static values
- Dirty tracking: constants never change, always elidable

`Reference` is the compiled form of expression strings. The expression resolver parses `"= nodes.leader.opacity"` into a `PropertyRef` AST stored in the `Anim` tree.

### 5.2 Interpolation Modes

```rust
enum InterpMode {
    Hold,           // step function
    Linear,         // existing
    CubicBezier {   // NEW
        x1: f64, y1: f64,
        x2: f64, y2: f64,
    },
    Spring {        // NEW: physics-based
        stiffness: f64,
        damping: f64,
        mass: f64,
    },
}
```

**CubicBezier**: Standard CSS `cubic-bezier(x1,y1,x2,y2)`. Replaces per-keyframe `Ease` enum for fine-grained control. `Ease` variants become **named presets** that expand to CubicBezier values:

```
┌───────────────────────┬──────────────────────────────┐
│ Named Ease            │ CubicBezier equivalent       │
├───────────────────────┼──────────────────────────────┤
│ linear                │ (0.00, 0.00, 1.00, 1.00)    │
│ ease                  │ (0.25, 0.10, 0.25, 1.00)    │
│ ease_in               │ (0.42, 0.00, 1.00, 1.00)    │
│ ease_out              │ (0.00, 0.00, 0.58, 1.00)    │
│ ease_in_out           │ (0.42, 0.00, 0.58, 1.00)    │
│ in_quad               │ (0.11, 0.00, 0.50, 0.00)    │
│ out_quad              │ (0.50, 1.00, 0.89, 1.00)    │
│ in_out_quad           │ (0.45, 0.00, 0.55, 1.00)    │
│ in_cubic              │ (0.32, 0.00, 0.67, 0.00)    │
│ out_cubic             │ (0.33, 1.00, 0.68, 1.00)    │
│ in_out_cubic          │ (0.65, 0.00, 0.35, 1.00)    │
│ in_quart              │ (0.50, 0.00, 0.75, 0.00)    │
│ out_quart             │ (0.25, 1.00, 0.50, 1.00)    │
│ in_out_quart          │ (0.76, 0.00, 0.24, 1.00)    │
│ in_expo               │ (0.70, 0.00, 0.84, 0.00)    │
│ out_expo              │ (0.16, 1.00, 0.30, 1.00)    │
│ in_out_expo           │ (0.87, 0.00, 0.13, 1.00)    │
│ in_back               │ (0.36, 0.00, 0.66, -0.56)   │
│ out_back              │ (0.34, 1.56, 0.64, 1.00)    │
│ in_out_back           │ (0.68, -0.6, 0.32, 1.60)    │
│ in_elastic            │ custom (not pure cubic)      │
│ out_elastic           │ custom (not pure cubic)      │
│ out_bounce            │ custom (piecewise)           │
└───────────────────────┴──────────────────────────────┘
```

Elastic and bounce are **not** cubic bezier representable. They stay as named variants with dedicated `apply(t)` implementations. JSON accepts both `"ease": "out_cubic"` and `"ease": [0.33, 1.0, 0.68, 1.0]`.

### 5.3 Spring Physics Solver

Replace the crude `target * (1 - e^(-rt)(1 + rt))` with a proper ODE solver:

```
Spring ODE:  x'' + (damping/mass)*x' + (stiffness/mass)*x = 0

Three regimes:
  ω₀ = sqrt(stiffness/mass)
  ζ  = damping / (2 * sqrt(stiffness * mass))

  ζ < 1  → underdamped  (oscillates, decays)
  ζ = 1  → critically damped (fastest non-oscillating)
  ζ > 1  → overdamped  (slow exponential approach)
```

Analytical closed-form solution for all three regimes. No numerical integration needed (springs are LTI systems). Evaluated per-frame from `(initial_value, target_value, stiffness, damping, mass, time_seconds)`.

Spring is usable **as an InterpMode on keyframes** (spring from value A to value B) or as a **Procedural** source (continuous spring responding to animated target).

### 5.4 Expr Combinators (expanded)

v0.2.1 Expr variants: `Delay`, `Speed`, `Reverse`, `Loop`, `Mix`.

v0.3 adds:

```rust
enum Expr<T> {
    // ... existing ...
    Delay { inner, by: u64 },
    Speed { inner, factor: f64 },
    Reverse { inner, duration: u64 },
    Loop { inner, period: u64, mode: LoopMode },
    Mix { a, b, t: Anim<f64> },

    // NEW
    Sequence { segments: Vec<(u64, Anim<T>)> },  // chain with explicit switch points
    Stagger { items: Vec<Anim<T>>, offset: Anim<f64>, base_delay: u64 },
    Map { inner: Anim<f64>, f: MappingFn },       // remap f64 output through a function
    Clamp { inner, min: T, max: T },
    Conditional { condition: Anim<f64>, threshold: f64, then: Anim<T>, else_: Anim<T> },
}
```

`Stagger` works on **any** `Anim<T>` (not just `f64` as in v0.2.1). The `offset` parameter is itself animated, enabling dynamic stagger spacing.

### 5.5 Lerp Trait Surface

```rust
trait Animatable: Lerp + Clone + Default + Serialize + Deserialize {
    fn is_constant_default(&self) -> bool;  // optimization: skip sampling
}
```

`ProcValue` becomes **optional** — not every animatable type needs procedural generation support. Types that want procedural support additionally implement `ProcValue`. This unblocks wavyte-std from adding types like `BorderRadius`, `Shadow`, `TextStyle` as animatable without requiring procedural implementations.

Core `Animatable` implementations:
`f64`, `f32`, `Vec2`, `Color`, `Transform2D`, `Rgba8Premul`

---

## 6. Effect Pipeline

### 6.1 Effect Definition Model

```rust
struct EffectInstance {
    kind: String,                              // registry key
    params: BTreeMap<String, AnimParam>,        // ALL params are animatable
}

enum AnimParam {
    Float(Anim<f64>),
    Vec2(Anim<Vec2>),
    Color(Anim<Color>),
    Bool(bool),           // static (not worth animating)
    String(String),       // static (enum selectors, etc.)
}
```

**Every numeric/color effect parameter is `Anim<T>`** by default. The After Effects rule: if it's a number, it's animatable.

JSON shorthand makes this painless:

```json
{ "blur": { "radius": 20 } }
// Deserialized as: params: { "radius": AnimParam::Float(Anim::Constant(20.0)) }

{ "blur": { "radius": { "keyframes": [
    { "frame": 0, "value": 20, "ease": "out_cubic" },
    { "frame": 30, "value": 0 }
]}}}
// Animated blur radius
```

### 6.2 Effect Registry

```rust
trait EffectDef: Send + Sync {
    fn kind(&self) -> &str;

    fn param_schema(&self) -> ParamSchema;

    fn validate(&self, params: &BTreeMap<String, AnimParam>) -> WavyteResult<()>;

    /// Resolve animated params at a specific frame, producing static snapshot
    fn resolve_params(&self, params: &BTreeMap<String, AnimParam>, ctx: SampleCtx)
        -> WavyteResult<ResolvedEffectParams>;

    /// Decompose into core render primitives
    fn compile(&self, resolved: &ResolvedEffectParams) -> WavyteResult<CompiledEffect>;
}

/// Output of compile: what the effect actually does in render plan terms
struct CompiledEffect {
    inline: InlineFx,              // opacity/transform adjustments
    passes: Vec<PassFx>,           // offscreen render passes
    masks: Vec<MaskOp>,            // mask/clip operations
}
```

**Registry** is a `Vec<Box<dyn EffectDef>>` built at compile time. Core registers built-in effects. wavyte-std appends its effects via a `register_std_effects(registry)` call.

### 6.3 Core Effects (v0.3)

| Effect | Category | Params (all animatable unless noted) |
|---|---|---|
| `opacity_mul` | Inline | `value: f64` |
| `transform_post` | Inline | `affine: [f64; 6]` or structured |
| `blur` | Pass | `radius: f64`, `sigma: f64` |
| `brightness` | Pass (color matrix) | `value: f64` (1.0 = identity) |
| `contrast` | Pass (color matrix) | `value: f64` |
| `saturate` | Pass (color matrix) | `value: f64` |
| `hue_rotate` | Pass (color matrix) | `degrees: f64` |
| `tint` | Pass (color matrix) | `color: Color`, `amount: f64` |
| `color_matrix` | Pass | `matrix: [f64; 20]` |
| `drop_shadow` | Multi-pass | `offset: Vec2`, `blur: f64`, `color: Color`, `spread: f64` |
| `inner_shadow` | Multi-pass | same as drop_shadow |
| `border` | Draw | `width: f64`, `color: Color`, `radius: f64` |
| `clip_rect` | Mask | `rect: Rect`, `radius: f64` |
| `clip_path` | Mask | `path: String` (SVG path d) |

wavyte-std adds higher-level effects (glow, neon, glass, gradient-map, etc.) by composing these primitives.

### 6.4 Color Matrix Pipeline

Multiple color effects (`brightness`, `contrast`, `saturate`, `hue_rotate`, `tint`) each produce a `4×5` color matrix. When stacked, they **multiply into a single matrix** before application. One pass, one matrix multiplication per pixel. This is the same approach as CSS/SVG filter chains.

```
Combined = brightness_mtx × contrast_mtx × saturate_mtx × hue_mtx × tint_mtx
```

Applied in the compiler: if an effect chain has N color-matrix effects, they fold into a single `PassFx::ColorMatrix([f64; 20])`.

### 6.5 RenderPlan Changes

New pass types needed:

```rust
enum PassFx {
    Blur { radius_px: u32, sigma: f32 },
    ColorMatrix { matrix: [f32; 20] },          // NEW
    MaskApply { mask_surface: SurfaceId,        // NEW
                mode: MaskMode,
                inverted: bool },
    DropShadow { offset: Vec2, blur_radius: u32,// NEW
                 sigma: f32, color: Rgba8Premul },
}

enum CompositeOp {
    Over { src, opacity },
    Crossfade { a, b, t },
    Wipe { a, b, t, dir, soft_edge },
    MaskComposite { content: SurfaceId,         // NEW
                    mask: SurfaceId,
                    mode: MaskMode,
                    inverted: bool },
}
```

---

## 7. Transform System

### 7.1 Transform2D (unchanged concept, richer)

```rust
struct Transform2D {
    translate: Vec2,
    rotation_rad: f64,
    scale: Vec2,
    anchor: Vec2,
    skew: Vec2,        // NEW: skew_x, skew_y in radians
}
```

Composition order: `T(translate) × T(anchor) × R(rotation) × K(skew) × S(scale) × T(-anchor)`

### 7.2 Transform Inheritance in Scene Tree

When evaluating a `Collection(Group)` node, the group's resolved `Affine` transform is **inherited** by all children:

```
child_world_transform = parent_world_transform × child_local_transform
```

Opacity also cascades multiplicatively:

```
child_effective_opacity = parent_opacity × child_opacity
```

This is standard scene graph behavior (identical to HTML/CSS, After Effects, Figma). The evaluator walks the tree depth-first, maintaining a transform/opacity stack.

---

## 8. Layout Engine

### 8.1 Taffy Integration

Replace the custom solver with [Taffy](https://github.com/DioxusLabs/taffy), a mature Rust implementation of CSS Flexbox and Grid.

**Each `Collection` node with layout enabled** maps to a Taffy node. Children map to child Taffy nodes. Layout properties on the node:

```rust
struct LayoutProps {
    display: Display,           // Flex | Grid | None
    direction: FlexDirection,   // Row | Column | RowReverse | ColumnReverse
    wrap: FlexWrap,
    justify_content: JustifyContent,
    align_items: AlignItems,
    align_content: AlignContent,
    gap: Vec2,
    padding: Edges,
    // Per-child overrides via child node's LayoutProps:
    flex_grow: f64,
    flex_shrink: f64,
    flex_basis: Dimension,
    align_self: AlignSelf,
    size: Size<Dimension>,      // width, height (auto, px, percent)
    min_size: Size<Dimension>,
    max_size: Size<Dimension>,
    margin: Edges,
    position: Position,         // Relative | Absolute
}
```

### 8.2 Per-Frame Layout Solving

Layout runs **inside the evaluation loop**, not as a one-shot prepass. Why:

- Text content can change per frame (counter, typewriter effect) → size changes → layout reflows
- Animated `size` or `flex_basis` parameters drive layout transitions
- `Switch` collections change which child is visible → layout should adapt

Taffy is fast (~microseconds for typical trees). With 10ms/frame budget and most of that in rendering, layout is negligible.

**Flow**:

1. Walk scene tree, measure leaf node intrinsic sizes (from prepared assets)
2. Build/update Taffy tree (cached between frames; only rebuild if structure changed)
3. Compute layout → produces `(x, y, width, height)` for each node
4. Inject layout positions as translation offsets into the evaluation transform stack

### 8.3 Mapping from v0.2.1

| v0.2.1 LayoutMode | v0.3 LayoutProps |
|---|---|
| `Absolute` | `display: None` or no LayoutProps |
| `HStack` | `display: Flex, direction: Row` |
| `VStack` | `display: Flex, direction: Column` |
| `Grid` | `display: Grid, grid_template_columns: repeat(N, 1fr)` |
| `Center` | `display: Flex, justify_content: Center, align_items: Center` |

---

## 9. Evaluator Redesign

### 9.1 Recursive Scene Tree Walk

v0.2.1 evaluator does a flat scan over `tracks → clips`. v0.3 evaluator does a **recursive depth-first walk** of the scene tree.

```
eval_node(node, parent_ctx: EvalContext) → EvalResult
  │
  ├── Check node.range against current frame → skip if out of range
  │
  ├── Resolve expressions (from topologically sorted dependency order)
  │
  ├── Sample node.transform → Anim<Transform2D>.sample(ctx) → Transform2D
  ├── Compute world_transform = parent_ctx.world_transform × local.to_affine()
  ├── Sample node.opacity → clamp(0,1) × parent_ctx.opacity
  │
  ├── match node.kind:
  │   ├── Leaf → emit EvaluatedLeafNode
  │   ├── Collection(Group) →
  │   │     for child in children: eval_node(child, updated_ctx)
  │   │     apply group mask if present
  │   ├── Collection(Sequence) →
  │   │     compute which child is active at current local frame
  │   │     eval_node(active_child, remapped_ctx)
  │   ├── Collection(Stack) →
  │   │     for child in children: eval_node(child, updated_ctx)
  │   │     (same as Group but no shared mask by default)
  │   ├── Collection(Switch) →
  │   │     sample active index → eval_node(children[idx], updated_ctx)
  │   └── CompRef →
  │         resolve referenced composition
  │         eval as nested scene tree with its own time remapping
  │
  ├── Resolve effects with animated params (sample each AnimParam)
  ├── Resolve transitions (in/out windows)
  │
  └── Return EvalResult { nodes: Vec<EvaluatedLeafNode>, masks, effects }
```

### 9.2 EvaluatedGraph (v0.3)

```rust
struct EvaluatedGraph {
    frame: FrameIndex,
    /// Flat list of leaf nodes in draw order (depth-first), with fully resolved transforms/effects
    leaves: Vec<EvaluatedLeaf>,
    /// Group operations that affect leaf ranges (masks, group opacity, group effects)
    groups: Vec<EvaluatedGroup>,
}

struct EvaluatedLeaf {
    node_id: String,
    asset_key: String,
    world_transform: Affine,
    opacity: f64,
    blend: BlendMode,
    source_time_s: Option<f64>,
    effects: Vec<ResolvedEffect>,
    transition_in: Option<ResolvedTransition>,
    transition_out: Option<ResolvedTransition>,
    /// Index range in `leaves` this leaf belongs to (for group scoping)
    group_stack: Vec<usize>,
}

struct EvaluatedGroup {
    node_id: String,
    /// Range of leaf indices this group encompasses
    leaf_range: Range<usize>,
    mask: Option<ResolvedMask>,
    effects: Vec<ResolvedEffect>,
    opacity: f64,
}
```

The compiler uses `EvaluatedGroup` to know when to render a set of leaves into a temporary surface (for group mask or group effects), then composite back.

### 9.3 ResolvedEffect (v0.3)

```rust
struct ResolvedEffect {
    kind: String,
    /// Fully sampled static params (no more Anim, all values resolved for this frame)
    params: BTreeMap<String, ResolvedParam>,
}

enum ResolvedParam {
    Float(f64),
    Vec2(Vec2),
    Color(Rgba8Premul),
    Bool(bool),
    String(String),
}
```

The evaluator samples every `AnimParam` in every `EffectInstance` at the current frame, producing fully static `ResolvedParam` values. The compiler never sees `Anim<T>` — it works with resolved snapshots.

---

## 10. Transition System (expanded)

### 10.1 Core Transitions

| Transition | Params |
|---|---|
| `crossfade` | (none) |
| `wipe` | `dir`, `soft_edge` |
| `slide` | `dir`, `push: bool` (slide in vs push out) |
| `zoom` | `from_scale: f64`, `origin: Vec2` |
| `iris` | `shape: Circle\|Rect\|Diamond`, `origin: Vec2`, `soft_edge` |

`slide`, `zoom`, `iris` are new in v0.3. They decompose into transform animations + mask operations in the compiler. The transition registry follows the same trait pattern as effects:

```rust
trait TransitionDef: Send + Sync {
    fn kind(&self) -> &str;
    fn compile(&self, resolved_params: &ResolvedParams, progress: f64)
        -> WavyteResult<CompiledTransition>;
}

struct CompiledTransition {
    out_transform: Option<Affine>,
    in_transform: Option<Affine>,
    out_opacity: Option<f32>,
    in_opacity: Option<f32>,
    mask: Option<MaskOp>,
    composite: CompositeStrategy,
}
```

---

## 11. Blend Modes

v0.2.1 has only `Normal`. v0.3 core set:

```rust
enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    SoftLight,
    HardLight,
    Difference,
    Exclusion,
}
```

Each implemented as a pixel-level blend function in `effects/composite.rs`. These are the **Porter-Duff + separable blend modes** from the Compositing and Blending Level 1 spec. Non-separable modes (Hue, Saturation, Color, Luminosity) are deferred to wavyte-std.

---

## 12. Registry Architecture

### 12.1 Central Registry

```rust
struct Registry {
    effects: Vec<Box<dyn EffectDef>>,
    transitions: Vec<Box<dyn TransitionDef>>,
    // Assets remain a closed enum in core (extended by adding variants)
    // BlendModes remain a closed enum in core
}

impl Registry {
    fn new() -> Self { /* register all core built-ins */ }
    fn register_effect(&mut self, def: Box<dyn EffectDef>);
    fn register_transition(&mut self, def: Box<dyn TransitionDef>);
    fn find_effect(&self, kind: &str) -> Option<&dyn EffectDef>;
    fn find_transition(&self, kind: &str) -> Option<&dyn TransitionDef>;
}
```

### 12.2 Static Dispatch in Practice

wavyte-std registers its effects at library initialization:

```rust
// wavyte-std/src/lib.rs
pub fn create_registry() -> wavyte::Registry {
    let mut reg = wavyte::Registry::new();  // core built-ins
    reg.register_effect(Box::new(effects::Glow));
    reg.register_effect(Box::new(effects::GlassMorphism));
    reg.register_transition(Box::new(transitions::MorphTransition));
    reg
}
```

Python/TS bindings receive a registry from wavyte-std and use it when calling `render()`. Users who want custom effects write Rust in wavyte-std and recompile — the FFI layer does not support dynamic effect registration. This is the accepted tradeoff.

### 12.3 Shared Primitive Heuristic

Decision rule for core vs std:

> **If a render primitive appears in 2+ distinct high-level std abstractions → it belongs in core.**

Applied to the v0.3 surface:

| Primitive | Used in | Location |
|---|---|---|
| Gaussian blur | blur effect, drop shadow, glow, glass | core |
| Color matrix | brightness, contrast, saturate, hue_rotate, tint | core |
| Alpha mask | mask, iris transition, shape clip | core |
| Rounded rect clip | card layouts, frame borders, UI elements | core |
| Drop shadow | drop_shadow effect, card preset, text shadow | core |
| Glow | only glow effect | std |
| Glass morphism | only glass effect | std |
| Gradient map | only gradient_map effect | std |

---

## 13. wavyte-stitch Server Design

### 13.1 Role

wavyte-stitch is a **headless server binary** exposing the wavyte core + std over HTTP/WebSocket/SSE. Any GUI (web-based, Electron, native) can be built on top.

### 13.2 API Surface

```
POST   /composition          → Load/replace composition JSON
PATCH  /composition/node/:id → Partial update of a single node
GET    /frame/:n             → Render and return frame N as PNG
WS     /preview              → Stream frames as composition changes
POST   /render               → Batch render to MP4 (returns job ID)
GET    /render/:job          → Poll render status / download result
GET    /schema               → Return JSON Schema for composition format
POST   /validate             → Validate composition JSON, return errors
```

### 13.3 Incremental Rendering Hooks

For real-time preview, the server needs to minimize re-work when a user tweaks one parameter.

**Design-now decisions** (implemented in v0.3 core, used by stitch later):

1. **Expression dependency graph is cached**: Changing node A's opacity doesn't invalidate node B unless B references A.

2. **Layout is incrementally computable**: Taffy supports incremental re-layout when only one node's size changes.

3. **Prepared assets are cached with content-addressing**: Changing a text node's content only re-prepares that one text asset.

4. **EvaluatedGraph is diffable**: Two consecutive evaluated graphs can be compared to determine which surfaces need re-rendering. Nodes whose transform/opacity/effects haven't changed between frames produce the same surface → skip re-render.

**Not in v0.3 scope** (deferred to v0.5): actual incremental pipeline implementation. v0.3 lays the **structural foundation** (node identity, expression DAG, deterministic hashing) so that stitch can build on it.

---

## 14. Consumer API Surfaces

### 14.1 wavyte (core) Public API

```rust
// Construction
Composition::from_json(json: &str, registry: &Registry) -> WavyteResult<Composition>
Composition::to_json(&self) -> WavyteResult<String>
validate_schema(json: &str) -> WavyteResult<()>

// Preparation
PreparedAssetStore::prepare(comp: &Composition, root: impl AsRef<Path>) -> WavyteResult<Self>

// Rendering
render_frame(comp, frame, backend, assets, registry) -> WavyteResult<FrameRGBA>
render_to_mp4(comp, out_path, opts, backend, assets, registry) -> WavyteResult<RenderStats>

// Registry
Registry::new() -> Self      // core built-ins only
```

### 14.2 wavyte-std Public API

```rust
// Extended registry
wavyte_std::create_registry() -> Registry  // core + std effects/transitions

// Builder DSL (ergonomic Rust API)
SceneBuilder::new(canvas, fps, duration)
    .variable("title", "Hello")
    .asset("bg", SolidRect { ... })
    .group("main", |g| {
        g.node("title", text_asset, range)
            .opacity(fade_in(15))
            .transform(slide_from_bottom(30))
        .node("bg", "bg", range)
    })
    .build()

// Animation presets
fade_in(duration_frames) -> Anim<f64>
fade_out(duration_frames) -> Anim<f64>
slide_from(direction, distance, duration, ease) -> Anim<Transform2D>
scale_in(from, to, duration, ease) -> Anim<Transform2D>
spring_to(target, stiffness, damping) -> Anim<f64>
typewriter(text, chars_per_frame) -> Anim<String>  // for text content animation

// Effect presets
glow(radius, color, intensity) -> EffectInstance
glass(blur_radius, tint, saturation) -> EffectInstance
text_shadow(offset, blur, color) -> EffectInstance
```

### 14.3 wavyte-py / wavyte-ts API

Both consume JSON. The Python example:

```python
import wavyte

comp = {
    "version": "0.3",
    "canvas": {"width": 1920, "height": 1080},
    "fps": {"num": 30, "den": 1},
    "duration": 150,
    "variables": {"title": "Hello World"},
    "assets": {
        "bg": {"SolidRect": {"width": 1920, "height": 1080, "color": "#1a1a2e"}},
        "title_text": {"Text": {"text": "= vars.title", ...}}
    },
    "root": {
        "id": "root",
        "kind": {"Collection": {"mode": "Stack", "children": [
            {"id": "bg", "kind": {"Leaf": {"asset": "bg"}}, "range": [0, 150]},
            {"id": "title", "kind": {"Leaf": {"asset": "title_text"}},
             "range": [0, 150],
             "opacity": {"keyframes": [
                 {"frame": 0, "value": 0, "ease": "out_cubic"},
                 {"frame": 20, "value": 1}
             ]}}
        ]}}
    }
}

# Validate
errors = wavyte.validate(comp)

# Render single frame
frame = wavyte.render_frame(comp, frame=0, assets_root="./assets")

# Render to MP4
wavyte.render_to_mp4(comp, "output.mp4", assets_root="./assets")

# Batch: render 100 videos with different variables
for item in data:
    comp["variables"]["title"] = item["title"]
    wavyte.render_to_mp4(comp, f"output_{item['id']}.mp4", assets_root="./assets")
```

---

## 15. Performance Strategy (CPU-only)

### 15.1 Budget

Target: **< 33ms per frame** at 1080p on 4 vCPU (30fps real-time preview).  
Current v0.2.1: ~10ms per frame for typical renders. v0.3 adds scene tree walk, per-frame layout, expression evaluation, animated effect params. Estimated overhead: +2-5ms.

### 15.2 Optimization Levers

| Technique | Impact | Complexity |
|---|---|---|
| `Anim::Constant` fast path | High — skip keyframe search for static values | Low |
| Expression AST caching | High — parse once, evaluate many | Low |
| Color matrix folding | Medium — N color effects → 1 pass | Low |
| Taffy layout caching | Medium — skip re-layout if structure unchanged | Low |
| Parallel frame rendering (existing rayon) | High — already in v0.2.1 | Done |
| Static frame elision (existing) | Medium — skip duplicate frames | Done |
| SIMD color matrix application | Medium — 4 channels × matrix in one go | Medium |
| Lazy surface allocation | Low — don't allocate surfaces for invisible nodes | Low |

### 15.3 Frame Fingerprinting (v0.3)

Extend v0.2.1 fingerprinting to cover scene tree. The fingerprint now includes:
- All leaf node resolved properties
- Group mask definitions
- Group effect resolved params
- Expression-resolved values

This preserves static frame elision accuracy for the richer v0.3 model.

---

## 16. Module Structure

```
wavyte/src/
├── lib.rs
├── schema/
│   ├── mod.rs
│   ├── version.rs          // version parsing, migration hooks
│   ├── validate.rs         // JSON Schema validation
│   └── shorthand.rs        // custom Serde deserializers for sugar
├── expression/
│   ├── mod.rs
│   ├── parser.rs           // expression string → AST
│   ├── resolver.rs         // dependency graph, topo sort
│   └── eval.rs             // per-frame expression evaluation
├── scene/
│   ├── mod.rs
│   ├── node.rs             // Node, NodeKind, CollectionMode
│   ├── composition.rs      // Composition, Variables
│   ├── mask.rs             // MaskDef, MaskMode, MaskSource
│   └── dsl.rs              // Rust builder DSL
├── animation/
│   ├── mod.rs
│   ├── anim.rs             // Anim<T>, Keyframes, Constant, Reference
│   ├── ease.rs             // Ease presets + CubicBezier
│   ├── spring.rs           // Proper spring ODE solver
│   ├── interp.rs           // InterpMode, interpolation dispatch
│   ├── ops.rs              // Combinators (delay, loop, stagger, etc.)
│   └── proc.rs             // Procedural sources
├── assets/
│   ├── mod.rs
│   ├── decode.rs
│   ├── media.rs
│   ├── store.rs
│   ├── svg_raster.rs
│   ├── generated.rs        // SolidRect, Gradient, Noise rendering
│   └── color.rs            // Color type, Lerp, conversions
├── effects/
│   ├── mod.rs
│   ├── registry.rs         // EffectDef trait, Registry
│   ├── blur.rs
│   ├── color_matrix.rs     // brightness, contrast, saturate, hue_rotate, tint
│   ├── shadow.rs           // drop_shadow, inner_shadow
│   ├── mask_ops.rs         // clip_rect, clip_path, alpha/luma mask
│   ├── composite.rs        // blend modes + compositing
│   └── transitions.rs      // TransitionDef trait + core transitions
├── layout/
│   ├── mod.rs
│   └── taffy_bridge.rs     // Taffy integration, per-frame solve
├── eval/
│   ├── mod.rs
│   ├── evaluator.rs        // Recursive scene tree evaluator
│   └── context.rs          // EvalContext, transform/opacity stack
├── compile/
│   ├── mod.rs
│   ├── plan.rs             // RenderPlan, PassFx, DrawOp (expanded)
│   ├── compiler.rs         // EvaluatedGraph → RenderPlan (with groups)
│   └── fingerprint.rs
├── render/
│   ├── mod.rs
│   ├── backend.rs
│   ├── cpu.rs
│   ├── passes.rs
│   └── pipeline.rs
├── encode/
│   └── ffmpeg.rs
├── foundation/
│   ├── core.rs
│   ├── error.rs
│   └── math.rs
└── transform/
    ├── affine.rs
    ├── linear.rs
    └── non_linear.rs
```

---

## 17. Implementation Order

Sequenced so each phase produces a compilable, testable intermediate. No big-bang integration. 

Current v0.2.1 core is ~5,000 LOC. v0.3 rewrites most of it and adds substantially.


### Phase 1: Foundation Types — ~1,000 LOC

- [ ] `Color` type with Lerp, JSON shorthand
- [ ] `Anim::Constant` variant + custom Serde deserializer
- [ ] `InterpMode::CubicBezier` + 30 easing presets
- [ ] `Transform2D` with `skew` field
- [ ] `Animatable` trait replacing `Lerp + Clone + ProcValue` 
- [ ] Spring ODE solver — 3-regime analytical
- [ ] `AnimParam` type (animatable effect parameters)

**Test gate**: Unit tests for all new types. JSON round-trip for shorthand.

### Phase 2: Scene Graph Model

- [ ] `Node`, `NodeKind`, `CollectionMode`
- [ ] `MaskDef`, `MaskMode`, `MaskSource`
- [ ] `Composition` with `root: Node`, variables, validation
- [ ] Generated asset variants: `SolidRect`, `Gradient`, `Noise`
- [ ] Schema validation (`version: "0.3"`, structural checks)
- [ ] Composition JSON serde + all shorthand
- [ ] Variables system 

**Test gate**: Rebuild v0.2.1 example compositions in new model. JSON schema roundtrip.

### Phase 3: Expression Engine

- [ ] Recursive-descent parser (string → AST)
- [ ] Dependency graph builder + topological sort + cycle detection
- [ ] Expression evaluator (arithmetic, built-in functions, property refs) 
- [ ] `Anim::Reference` integration

**Test gate**: Parser unit tests. Cross-node reference chains. Cycle rejection.

### Phase 4: Evaluator Rewrite

- [ ] Recursive scene tree walker
- [ ] Transform/opacity inheritance stack + EvalContext 
- [ ] Sequence/Switch/CompRef time remapping
- [ ] Expression resolution during evaluation 
- [ ] `EvaluatedGraph` with `leaves` + `groups` 
- [ ] Animated effect param resolution → `ResolvedEffect` 

**Test gate**: Complex scene trees evaluate correctly. Transform inheritance verified.

### Phase 5: Layout Engine 

- [ ] Taffy integration bridge
- [ ] `LayoutProps` mapping to Taffy styles
- [ ] Per-frame solving in evaluator loop
- [ ] Intrinsic size measurement for all asset types

**Test gate**: Flexbox/Grid correctness against CSS reference behavior.

### Phase 6: Effect Pipeline + Registry 

- [ ] `EffectDef` / `TransitionDef` traits + `Registry`
- [ ] Color matrix pipeline (brightness, contrast, saturate, hue_rotate, tint)
- [ ] `PassFx::ColorMatrix` in plan + CPU backend 
- [ ] Drop shadow multi-pass 
- [ ] Mask operations in plan + CPU backend 
- [ ] Group rendering (children → temp surface → group effects/mask) 
- [ ] 12 blend modes in CPU compositor 
- [ ] New transitions: slide, zoom, iris 

**Test gate**: Visual regression for each effect. Mask correctness. Blend mode correctness.

### Phase 7: Compiler Rewrite 

- [ ] `EvaluatedGroup` → temp surface + group effects 
- [ ] Mask compilation → surface + `MaskComposite` ops 
- [ ] Color matrix folding optimization 
- [ ] Fingerprinting for scene tree model 

**Test gate**: Surface count verification. Fingerprint stability across runs.

### Phase 8: Pipeline Integration

- [ ] `render_frame` / `render_to_mp4` updated with registry param
- [ ] Parallel rendering adapted to new model

**Test gate**: End-to-end JSON → MP4. Pixel-compare where applicable.

### Phase 9: wavyte-std Bootstrap (separate crate)

- [ ] Builder DSL (SceneBuilder, etc.) 
- [ ] Animation presets (fade, slide, scale, spring) 
- [ ] Effect presets (glow, glass, text shadow) 
- [ ] Layout presets (card, grid gallery, centered overlay)
- [ ] Template system (composition factories + variable injection)
- [ ] 10+ reference templates as examples

**Test gate**: All templates render correctly. wavyte-py can consume JSON produced by std.

### Critical Path

```
P1 ──→ P2 ──→ P4 ──→ P6 ──→ P7 ──→ P8
  └──→ P3 ──┘    └──→ P5 (parallel)
                       P9 can start at P6
```


---

## 18. JSON Schema Reference (abbreviated)

```json
{
  "$schema": "https://wavyte.dev/schemas/v0.3.json",
  "version": "0.3",
  "canvas": { "width": 1920, "height": 1080 },
  "fps": { "num": 30, "den": 1 },
  "duration": 300,
  "seed": 42,
  "variables": {
    "title": "My Video",
    "accent_color": "#ff5500"
  },
  "assets": {
    "bg": { "SolidRect": { "width": 1920, "height": 1080, "color": "#1a1a2e" } },
    "logo": { "Image": { "source": "assets/logo.png" } },
    "title_text": { "Text": {
      "text": "= vars.title",
      "font_source": "fonts/Inter-Bold.ttf",
      "size_px": 64,
      "color": "= vars.accent_color"
    }}
  },
  "root": {
    "id": "root",
    "kind": { "Collection": { "mode": "Stack", "children": [
      {
        "id": "background",
        "kind": { "Leaf": { "asset": "bg" } },
        "range": [0, 300]
      },
      {
        "id": "content_group",
        "kind": { "Collection": { "mode": "Group", "children": [
          {
            "id": "logo_node",
            "kind": { "Leaf": { "asset": "logo" } },
            "range": [0, 300],
            "transform": {
              "translate": [100, 50],
              "scale": 0.5
            },
            "opacity": { "keyframes": [
              { "frame": 0, "value": 0, "ease": "out_expo" },
              { "frame": 20, "value": 1 }
            ]}
          },
          {
            "id": "title_node",
            "kind": { "Leaf": { "asset": "title_text" } },
            "range": [10, 300],
            "transform": {
              "translate": [100, 200]
            },
            "opacity": { "keyframes": [
              { "frame": 0, "value": 0, "ease": "out_cubic" },
              { "frame": 25, "value": 1 }
            ]},
            "effects": [
              { "drop_shadow": { "offset": [3, 3], "blur": 8, "color": "#00000080" } }
            ]
          }
        ]}},
        "range": [0, 300],
        "mask": {
          "source": { "Shape": { "RoundedRect": { "width": 1720, "height": 980, "radius": 20 } } },
          "mode": "Alpha"
        },
        "transform": { "translate": [100, 50] }
      }
    ]}}
  }
}
```

---

## 19. Success Criteria

v0.3 is done when:

1. **Scene tree works end-to-end**: Nested groups with transform inheritance, masks, and group effects render correctly.
2. **Expression engine resolves cross-node references**: Node A's position can drive Node B's position.
3. **All effect params are animatable**: Blur radius, shadow offset, tint color animate smoothly.
4. **Layout engine produces correct Flexbox/Grid layouts** per-frame.
5. **30 easing presets + cubic bezier + spring** all work with visual correctness.
6. **JSON shorthand is ergonomic**: A simple composition is < 50 lines of JSON.
7. **wavyte-std can build 10+ production templates** without requiring core changes.
8. **Performance stays under 33ms/frame** at 1080p on 4 vCPU for typical compositions (< 50 nodes, < 10 effects).
9. **JSON schema validates and catches errors** before render.
10. **Expression cycles are detected** at validation time, not at render time.