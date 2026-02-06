# Wavyte v0.1 — EXPLANATION.md (deep codebase walkthrough)

This document is a deliberately **ASCII-heavy**, “explain every moving part” walkthrough of the
entire `src/` tree (≈7.8k LoC).

Goals:

1. Make a new developer productive without tribal knowledge.
2. Explain **every file**, **every struct/enum/function**, and every meaningful code block.
3. Explain the **data flow** and **control flow** between phases:
   - Evaluate → Compile → Render → (optional) Encode
4. Provide enough context to support **major refactors** without losing correctness.

Non-goals:

- This is not API docs (that’s `README.md` + rustdoc).
- This is not a design proposal; it describes “what exists now”.

Important repo convention:

- This file is intentionally **not git-tracked** (repo ignores `*.md` except `README.md`).

---

## How to read this doc

The walkthrough is presented in **dependency order** (leaf-ish modules first), and `src/lib.rs`
last.

Reality check (Rust module cycles):

Two module pairs form unavoidable mutual dependencies in v0.1:

1. `src/anim.rs` ↔ `src/anim_proc.rs`
   - `anim.rs` defines `SampleCtx`, which `anim_proc.rs` needs.
   - `anim.rs` also depends on `Procedural<T>`/`ProcValue`, which are defined in `anim_proc.rs`.
2. `src/assets.rs` ↔ `src/assets_decode.rs`
   - `assets.rs` defines prepared asset structs like `PreparedImage`.
   - `assets_decode.rs` implements decoding into those structs.
   - `assets.rs` also calls `assets_decode` to decode bytes.

We still present a stable “best-effort” order, and when we hit a cycle we explicitly point to the
counterpart module.

---

## Module order (best-effort)

1. `src/error.rs`
2. `src/core.rs`
3. `src/anim_ease.rs`
4. `src/anim.rs`  (cycle with `anim_proc.rs`)
5. `src/anim_proc.rs`
6. `src/anim_ops.rs`
7. `src/transitions.rs`
8. `src/model.rs`
9. `src/fx.rs`
10. `src/eval.rs`
11. `src/assets.rs` (cycle with `assets_decode.rs`)
12. `src/assets_decode.rs`
13. `src/svg_raster.rs`
14. `src/blur_cpu.rs`
15. `src/composite_cpu.rs`
16. `src/compile.rs`
17. `src/render_passes.rs`
18. `src/render_cpu.rs`
19. `src/render_vello.rs` (only built with `--features gpu`)
20. `src/render.rs`
21. `src/encode_ffmpeg.rs`
22. `src/pipeline.rs`
23. `src/dsl.rs`
24. `src/guide.rs` (pure rustdoc, but we still explain it structurally)
25. `src/bin/wavyte.rs` (CLI)
26. `src/lib.rs` (re-exports + module map)

---

## Global mental model (one-screen view)

Wavyte v0.1 pipeline:

```
              ┌──────────────────────────┐
              │        Composition       │
              │ (assets, tracks, clips)  │
              └────────────┬─────────────┘
                           │ frame index
                           v
┌─────────────────────────────────────────────────────────────────────┐
│ 1) Evaluate (src/eval.rs)                                            │
│   Composition + FrameIndex -> EvaluatedGraph                         │
│   - picks visible clips                                               │
│   - samples Anim<T>                                                   │
│   - resolves transitions/effects into typed instances                 │
└───────────────────────────────┬─────────────────────────────────────┘
                                │ evaluated graph
                                v
┌─────────────────────────────────────────────────────────────────────┐
│ 2) Compile (src/compile.rs)                                          │
│   EvaluatedGraph -> RenderPlan                                       │
│   - explicit surfaces                                                │
│   - ordered passes (scene/offscreen/composite)                        │
│   - DrawOp IR references AssetId (stable ids)                         │
└───────────────────────────────┬─────────────────────────────────────┘
                                │ render plan
                                v
┌─────────────────────────────────────────────────────────────────────┐
│ 3) Render (src/render_*.rs + src/render_passes.rs)                   │
│   RenderPlan + AssetCache -> FrameRGBA (premultiplied RGBA8)         │
│   - CPU default: vello_cpu + our CPU composites/effects              │
│   - GPU optional: vello/wgpu                                         │
│   - SVG: usvg parse + resvg rasterize (both CPU & GPU)               │
└───────────────────────────────┬─────────────────────────────────────┘
                                │ frames
                                v
┌─────────────────────────────────────────────────────────────────────┐
│ 4) Encode (src/encode_ffmpeg.rs + src/pipeline.rs)                   │
│   Stream frames to `ffmpeg` process, produce MP4                     │
└─────────────────────────────────────────────────────────────────────┘
```

Core design constraint repeated everywhere:

```
NO IO in renderers
  - render code must not touch filesystem/network
  - all external bytes go through AssetCache (FsAssetCache by default)
```

---

# CHUNK 1 (foundation) — `error`, `core`, `anim_*`

This chunk covers:

- `src/error.rs`
- `src/core.rs`
- `src/anim_ease.rs`
- `src/anim.rs` (partially; see note below)

Note: this doc is produced in **chunks** to keep generation manageable. Subsequent chunks will
append to this file until the requested 2–3k+ lines is reached.

---

## File: `src/error.rs`

### Purpose

`error.rs` defines a single crate-wide error type (`WavyteError`) and the ubiquitous
`WavyteResult<T>` alias.

This is the lowest-level internal module because almost every other module returns
`WavyteResult<T>` and constructs `WavyteError::*` variants.

### Key types

#### `pub type WavyteResult<T> = Result<T, WavyteError>;`

Why this exists:

- It reduces repetition (`-> Result<_, WavyteError>`) across the whole codebase.
- It makes refactors easier: if we ever need to change the top-level error type we do it in one
  place.

#### `pub enum WavyteError`

Variants:

- `Validation(String)`
  - Used when user input / composition data is invalid.
  - This includes path validation, out-of-bounds frame ranges, invalid FPS, etc.
- `Animation(String)`
  - Used for invalid animation expressions or keyframes.
- `Evaluation(String)`
  - Used when evaluation logic fails in a way that is not “user input invalid” (e.g., unknown
    `AssetId` in an asset cache).
- `Serde(String)`
  - Used for serialization/deserialization-level issues when we want a stable category.
- `Other(anyhow::Error)`
  - “Escape hatch” for external library errors and contextualized IO errors.
  - This is intentionally transparent: the source error’s message is preserved.

ASCII: how errors flow:

```
            ┌─────────────┐
            │ module code │
            └──────┬──────┘
                   │ returns WavyteResult<T>
                   v
┌─────────────────────────────┐
│ WavyteError::* categories    │
│  - Validation                │
│  - Animation                 │
│  - Evaluation                │
│  - Serde                     │
│  - Other(anyhow::Error)      │
└─────────────────────────────┘
```

#### Convenience constructors

Functions:

- `WavyteError::validation(msg)`
- `WavyteError::animation(msg)`
- `WavyteError::evaluation(msg)`
- `WavyteError::serde(msg)`

These do two things:

1. They standardize where these strings are created.
2. They make call-sites read like a category decision, not just a string allocation.

### Tests

#### `display_prefixes_are_stable`

Purpose: ensure `Display` strings include the category prefixes.

Why it matters:

- Downstream tools (like CLI or logs) often rely on these prefixes.
- Stability reduces churn when debugging.

#### `other_preserves_source`

Purpose: ensure `Other(anyhow::Error)` prints the underlying message.

Why it matters:

- `anyhow` carries cause chains and context; we want those to remain visible.

---

## File: `src/core.rs`

### Purpose

`core.rs` defines the fundamental math/time/pixel types used across the engine.

Important characteristics:

- These types are simple value objects.
- Most are `Copy`/`Clone`.
- Many are `serde`-serializable because they appear in the `Composition` model.

### Imports and re-exports

#### `use crate::error::{WavyteError, WavyteResult};`

`core` is “low-level”, but it still needs validation errors (e.g., invalid FPS).

#### `pub use kurbo::{Affine, BezPath, Point, Rect, Vec2};`

Wavyte uses `kurbo` as its geometry backbone.

Re-exporting here yields:

- One canonical geometry vocabulary for the whole crate: `wavyte::Affine`, `wavyte::Vec2`, etc.
- Less “dependency surface area” for downstream users: they can depend on `wavyte` and use the same
  types that the renderers use.

### Time primitives

#### `pub struct FrameIndex(pub u64);`

Meaning:

- 0-based index.
- “Absolute” within a `Composition` in most contexts.

Why newtype instead of `u64`:

- Stronger type-safety: you can’t accidentally pass “frames” where “pixels” are expected.
- Adds semantics and trait derives (`Ord`, etc.) in a controlled way.

#### `pub struct FrameRange { start, end }`

Semantics:

- Half-open range: `[start, end)` where `end` is **exclusive**.

Key methods:

- `FrameRange::new(start, end) -> WavyteResult<Self>`
  - Validates `start <= end`
  - This is an example of “validation belongs near the type”.
- `len_frames()`
  - Uses saturating subtraction because `u64` underflow is otherwise a footgun.
- `is_empty()`
  - True if `start == end`.
- `contains(FrameIndex)`
  - Implements the `[start, end)` contract.
- `clamp(FrameIndex)`
  - If range is empty, clamps to `start`.
  - Else clamps into `[start, end-1]` (since `end` is exclusive).
- `shift(delta: i64)`
  - Shifts both start/end forward/backward with saturation at 0.

ASCII: `FrameRange` boundaries:

```
start=2, end=5 means legal frames: 2,3,4

frame: 0 1 [2 3 4] 5 6
            ^^^^^
         contains() == true
```

### FPS (frames-per-second)

#### `pub struct Fps { num, den }`

This models rational FPS (`num/den`).

Examples:

- 30fps => `Fps { num: 30, den: 1 }`
- NTSC-ish 29.97fps => `Fps { num: 30000, den: 1001 }`

Methods:

- `Fps::new(num, den)` validates both are > 0.
- `as_f64()` gives `num/den` as floating point.
- `frame_duration_secs()` = `den/num`.
- `frames_to_secs(frames)` multiplies frames by duration.
- `secs_to_frames_floor(secs)` does `floor(secs * fps)`.

Why floor:

- Frame boundaries are discrete; it is safer to “not overshoot” by default.

### Canvas

#### `pub struct Canvas { width, height }`

Meaning:

- Output frame resolution in pixels.
- Used by the compiler and backends to size the primary surface.

### Pixel format: premultiplied alpha

#### `pub struct Rgba8Premul { r, g, b, a }`

Rule:

- RGB is already multiplied by alpha.

Why premultiplied:

- Makes “over” compositing mathematically stable.
- Avoids repeated per-pixel alpha multiplication in render passes.

Key constructors:

- `transparent()`
  - Returns `[0,0,0,0]`.
- `from_straight_rgba(r,g,b,a)`
  - Converts straight alpha → premultiplied alpha.
  - Uses rounding via `(c*a + 127)/255` (same pattern used in image premultiplication).

ASCII: premultiply intuition:

```
straight: (r=200,g=100,b=50,a=128)
premul:   (r≈100,g≈50,b≈25,a=128)  // each channel scaled by a/255
```

### Transforms

#### `pub struct Transform2D`

Fields:

- `translate: Vec2` (pixels in “canvas space”)
- `rotation_rad: f64`
- `scale: Vec2`
- `anchor: Vec2` (pivot in local coordinates)

Default:

- translate = (0,0)
- rotation = 0
- scale = (1,1)
- anchor = (0,0)

#### `Transform2D::to_affine() -> kurbo::Affine`

This constructs a single affine transform with a canonical order:

```
T(translate) * T(anchor) * R(rotation) * S(scale) * T(-anchor)
```

Interpretation:

- Move into world space with `translate`.
- Shift the local origin to the anchor pivot.
- Rotate around that anchor.
- Scale around that anchor.
- Shift back.

Common pitfall this avoids:

- Rotating around the top-left corner unintentionally.

### Tests

- `frame_range_contains_boundaries` validates `[start,end)` logic.
- `fps_frames_secs_roundtrip_floor` validates numeric stability for rational fps.
- `transform_to_affine_identity_and_translation` sanity-checks transform composition.

---

## File: `src/anim_ease.rs`

### Purpose

Defines easing curves used by keyframes and transitions.

`Ease::apply(t)` always clamps `t` to `[0,1]` and returns a value in `[0,1]`.

This makes easing robust when upstream code accidentally passes slight out-of-range values due to
floating-point math.

### `enum Ease`

Variants (v0.1 set):

- `Linear`
- `InQuad`, `OutQuad`, `InOutQuad`
- `InCubic`, `OutCubic`, `InOutCubic`

Each is a standard polynomial ease.

ASCII: `InOutQuad` shape (qualitative):

```
value
1.0 |           _______
    |        __/
    |     __/
    |  __/
0.0 |_/________________ time
     0       0.5       1
```

### Tests

- `endpoints_are_stable`: `apply(0)=0` and `apply(1)=1` for all variants.
- `monotonic_spot_check`: values increase from 0.25 → 0.5 → 0.75.

These tests are intentionally minimal:

- We care more about “obvious correctness” than perfectly characterizing curves.

---

## File: `src/anim.rs` (part 1)

### Purpose

`anim.rs` defines:

- The core `Anim<T>` abstraction.
- “Sample context” (`SampleCtx`) used to evaluate animations.
- `Keyframes<T>` (piecewise linear or hold).
- Expression wrappers (`Expr<T>`) like delay/speed/reverse/loop/mix.

Important: `Anim<T>` is used in:

- `model.rs` (`ClipProps` uses `Anim<Transform2D>` and `Anim<f64>` for opacity)
- `eval.rs` (sampling animations per-frame)
- `compile.rs` (only indirectly via evaluated values)

Cycle note:

- `Anim<T>` can contain `Procedural<T>`, which is defined in `src/anim_proc.rs`.
- Procedural evaluation needs `SampleCtx`, which is defined here.

### Imports

`anim.rs` imports:

- `anim_ease::Ease` (for keyframe interpolation)
- `anim_proc::{ProcValue, Procedural}` (procedural animation kinds)
- `core::{FrameIndex, Transform2D, Vec2}`
- `error::{WavyteError, WavyteResult}`

### `SampleCtx`

```
pub struct SampleCtx {
  frame: FrameIndex,      // global frame
  fps: Fps,               // global fps
  clip_local: FrameIndex, // frame - clip.start
  seed: u64,              // determinism seed
}
```

Why both `frame` and `clip_local`:

- Some effects want “absolute time in comp” (e.g. global noise).
- Others want time relative to a clip start (e.g. an entrance animation).

Why include `seed`:

- Procedural animation must be deterministic for a given input.
- This enables “random-looking but repeatable” motion.

### `trait Lerp`

`Lerp` is the contract for linear interpolation:

```
lerp(a,b,t) where t in [0,1]
```

Implemented for:

- `f64`, `f32`
- `Vec2`
- `Transform2D`
- `Rgba8Premul` (premultiplied; interpolates bytes with rounding)

Why implement `Lerp` for `Transform2D` directly:

- It allows `Anim<Transform2D>` to use the same keyframe mechanism as scalar values.
- Rotation uses direct linear interpolation in radians (no shortest-arc logic yet).

### `enum Anim<T>`

Variants:

- `Keyframes(Keyframes<T>)`
- `Procedural(Procedural<T>)`
- `Expr(Expr<T>)`

Constraints on `Anim<T>` methods:

`Anim<T>` methods require:

- `T: Lerp + Clone + ProcValue`

Meaning:

- It must be interpolatable (`Lerp`) for keyframes.
- It must be procedural-sampleable (`ProcValue`) for procedural nodes.
- It must be clonable for returning values cheaply.

### `Anim::constant(value)`

Builds a `Keyframes` with:

- one key at frame 0
- mode `Hold`

This is the “easy path” used throughout tests/examples to avoid verbose JSON.

### `Anim::sample(ctx)`

Dispatches by variant:

- `Keyframes.sample(ctx)`
- `Procedural.sample(ctx)`
- `Expr.sample(ctx)`

Important architectural point:

- Sampling is intentionally pure (no IO).
- Any nondeterminism must come only from `ctx.seed` (and even then is deterministic).

### `Anim::validate()`

Validation is “structural”:

- Keyframes validate keys sorted, etc.
- Expr validate parameters (period>0, factor>0, …).
- Procedural currently always OK (procedural kinds validate at sample time).

---

## Status

This file continues in the next chunk with:

- `Keyframes<T>` details
- `Expr<T>` details (delay/speed/reverse/loop/mix) and their sampling logic
- The remainder of `anim.rs` tests

---

<!-- CHUNK 1 END -->


# CHUNK 2 — finish `anim.rs` + cover `anim_proc.rs`

This chunk covers:

- The remainder of `src/anim.rs`
- All of `src/anim_proc.rs`

---

## File: `src/anim.rs` (part 2 — sampling + expressions + tests)

We already covered the “front half” of `anim.rs`:

- `SampleCtx`
- `Lerp`
- `Anim<T>` high-level shape

This section covers:

- `Keyframes<T>` internals
- `Expr<T>` validation + sampling
- The unit tests at the bottom

---

### `struct Keyframes<T>`

Code shape:

```
pub struct Keyframes<T> {
  pub keys: Vec<Keyframe<T>>, // sorted by frame
  pub mode: InterpMode,       // linear/hold
  pub default: Option<T>,
}
```

Interpretation:

- `keys` is the explicit keyframe list.
- `mode` chooses how to interpolate between two keys.
- `default` is only used when there are zero keys.

Why store `default` even though we usually create at least one key:

- JSON may represent “sparse” animations as a default + some overrides.
- Having a default enables a consistent “always sample” API even if keys are missing.

---

### `Keyframes<T>::validate()`

Validation rules:

1. `keys.is_empty() && default.is_none()` is invalid.
   - Without keys or default, sampling has no answer.
2. Keyframes must be sorted by `frame` (non-decreasing).
   - The sampler assumes ordering; unsorted keys would yield wrong results.

Error category:

- Uses `WavyteError::animation(...)` because this is an animation structure problem.

ASCII: sortedness requirement:

```
OK:  frame: 0, 10, 10, 42     (non-decreasing allowed)
BAD: frame: 0, 42, 10         (would break the sampler)
```

---

### `Keyframes<T>::sample(ctx)`

Core idea:

- Sampling uses `ctx.clip_local` (not `ctx.frame`).
- This makes animation time relative to a clip’s start.

Step-by-step sampling logic:

1. If `keys` is empty:
   - return `default` or error
2. Let `f = ctx.clip_local.0` (a `u64` frame number)
3. Compute `idx` using `partition_point(|k| k.frame <= f)`
   - After this:
     - keys `[0..idx)` are `<= f`
     - keys `[idx..]` are `> f`
4. If `idx == 0`:
   - `f` is before the first key → return first key value
5. If `idx >= keys.len()`:
   - `f` is after the last key → return last key value
6. Else:
   - we have two keys that bracket `f`:
     - `a = keys[idx-1]` (at or before)
     - `b = keys[idx]` (after)
   - compute normalized t in [0,1]
   - apply ease from key `a` (the “ease toward next key” convention)
   - if mode is `Hold`: return `a.value`
   - if mode is `Linear`: return `lerp(a.value, b.value, te)`

Important subtlety:

- The ease is stored per key, but is applied to the segment that starts at that key.
- That is: easing is attached to the “outgoing edge” of a key.

ASCII: key segments + ease attachment:

```
Key A (ease=InOutQuad) ----segment----> Key B
           ^ ease belongs here (outgoing)
```

Denominator edge-case:

- `denom = b.frame - a.frame` can be 0 if two keys share the same frame.
- In that case, the code returns `a.value` (and effectively ignores interpolation).

Why that behavior is acceptable in v0.1:

- Duplicate frames are allowed (non-decreasing, not strictly increasing).
- The semantics “later key at same frame wins” is approximated by the `idx` logic:
  - at exactly that frame, `idx` includes all keys `<= f`, so `a` tends toward the last key at that
    frame.

---

### `struct Keyframe<T>`

```
pub struct Keyframe<T> {
  pub frame: FrameIndex,
  pub value: T,
  pub ease: Ease,
}
```

Interpretation:

- `frame`: key time (in clip-local frames)
- `value`: value at that time
- `ease`: how to ease from this value toward the next key (if interpolating)

---

### `enum InterpMode`

- `Hold`:
  - “step function”: the value is constant until the next key boundary.
- `Linear`:
  - uses `T::lerp(a, b, eased_t)`

Why both exist:

- Many parameters want `Hold` (e.g. abrupt toggles, discrete states).
- Many want `Linear` (positions, opacity, etc.).

---

### `enum Expr<T>`

`Expr<T>` wraps another animation and remaps time / combines two animations.

Variants:

- `Delay { inner, by }`
- `Speed { inner, factor }`
- `Reverse { inner, duration }`
- `Loop { inner, period, mode }`
- `Mix { a, b, t }`

Think of `Expr<T>` as “functional combinators” over `Anim<T>`:

ASCII: expression node shape:

```
         Expr<T>
           |
        inner Anim<T>
           |
     (keyframes / procedural / ...)
```

---

### `Expr<T>::validate()`

Validate ensures expressions are well-defined:

- `Speed.factor > 0`
- `Reverse.duration > 0`
- `Loop.period > 0`
- recursively validates all children (`inner`, or `a/b/t`)

Why validate here:

- We want to catch structural errors early (composition validation time), not mid-render.

---

### `Expr<T>::sample(ctx)`

This is the most important part of `Expr<T>`.

#### Helper: `with_clip_local(ctx, clip_local)`

Purpose:

- Many expressions remap clip-local time.
- We must keep `ctx.frame` and `ctx.clip_local` consistent.

What the helper does:

1. Compute `delta = new_clip_local - old_clip_local`
2. Apply `delta` to `ctx.frame` with saturation
3. Set `ctx.clip_local = new_clip_local`

This preserves the invariant:

```
ctx.frame = (some global frame)
ctx.clip_local = ctx.frame - clip.start   (conceptually)
```

Even though the expression doesn’t know the clip start, it keeps both fields moving “together”.

ASCII: “frame” vs “clip_local” after remap:

```
before: frame=120, clip_local=30
remap:  clip_local=10  (delta=-20)
after:  frame=100, clip_local=10
```

#### Variant: `Delay { by }`

Behavior:

- Shifts animation right by `by` frames:
  - For the first `by` frames, sample the inner at 0.
  - Then sample inner at `f-by`.

Mapping:

```
mapped = max(f - by, 0)
```

#### Variant: `Speed { factor }`

Behavior:

- Scales time by a factor:
  - `factor=2.0` means “play inner twice as fast”.

Mapping:

```
mapped = floor(f * factor)
```

Why floor:

- Keeps mapping deterministic and stable.
- Avoids fractional frames (inner animations are discrete-time).

#### Variant: `Reverse { duration }`

Behavior:

- Plays the inner backwards over a fixed duration.

Mapping:

```
max = duration - 1
f_clamped = min(f, max)
mapped = max - f_clamped
```

Why clamp:

- If the outer animation lasts longer than `duration`, we stop at the reversed boundary instead of
  underflowing.

#### Variant: `Loop { period, mode }`

Behavior:

- Repeats inner animation over `period` frames.

Mode: `Repeat`

```
mapped = f % period
```

Mode: `PingPong`

Ping-pong means:

- time goes 0..period-1, then reverses back to 0, then repeats

Special case:

- If `period == 1`, ping-pong is always 0 (avoids dividing by zero / weird cycle math).

General case:

```
cycle = 2*(period - 1)
pos = f % cycle
mapped = pos if pos < period else cycle - pos
```

ASCII: ping-pong example with period=4:

```
f:      0 1 2 3 4 5 6 7 8 9 ...
mapped: 0 1 2 3 2 1 0 1 2 3 ...
```

#### Variant: `Mix { a, b, t }`

Behavior:

- Samples:
  - `tt = clamp(t.sample(ctx), 0, 1)`
  - `av = a.sample(ctx)`
  - `bv = b.sample(ctx)`
- Then returns `lerp(av, bv, tt)`

Why `t` is an `Anim<f64>`:

- `t` can itself be keyframed / procedural / expression-driven.

Important performance note:

- `Mix` samples three animations each time.
- If those animations are expensive expressions (deep trees), mixing becomes a hotspot.
- In v0.1 this is acceptable; later refactors might add memoization or DAG sharing.

---

### `anim.rs` tests (what they prove)

Helper `ctx(frame)`:

- Builds a `SampleCtx` where:
  - `frame == clip_local == frame`
  - `fps = 30/1`
  - `seed = 0`

This means tests are purely about “time mapping”, not clip offsets.

#### `keyframes_hold_is_constant_between_keys`

Ensures:

- With `InterpMode::Hold`, frames between keys return the previous key’s value.
- At exactly a keyframe boundary, the new value takes effect.

#### `keyframes_linear_interpolates`

Ensures:

- With linear mode and linear ease, midpoints interpolate as expected.

#### `expr_reverse_maps_frames`

Ensures:

- `Reverse` maps 0 → max and max → 0 (when inner uses hold mode).

---

## File: `src/anim_proc.rs`

### Purpose

`anim_proc.rs` adds “procedural” animation sources to complement keyframes.

In v0.1, procedural animations are:

- deterministic (seed-driven)
- purely functional (no IO)
- limited to a small set of scalar waveforms and derived vector forms

ASCII: where procedural fits:

```
Anim<T>
 ├─ Keyframes<T>
 ├─ Procedural<T>   <-- this module
 └─ Expr<T>
```

---

### Imports

`anim_proc.rs` depends on:

- `anim::SampleCtx` (cycle back to `anim.rs`)
- `core::{Fps, Transform2D, Vec2}`
- `error::{WavyteError, WavyteResult}`

Why it needs `Fps`:

- Several procedural functions are defined in “Hz” (cycles per second).
- We must convert frame count → seconds via `fps.frames_to_secs(frame)`.

---

### `struct Procedural<T>`

```
pub struct Procedural<T> {
  pub kind: ProceduralKind,
  _marker: PhantomData<T>,
}
```

Key idea:

- The procedural “kind” is independent of output type `T`.
- But we still want a `Procedural<f64>` vs `Procedural<Vec2>` typed at compile time.

`PhantomData<T>` exists to:

- carry `T` in the type system without storing a value
- keep Serde shape stable (marker is `#[serde(skip)]`)

Constructor:

- `Procedural::new(kind)` sets the kind and marker.

Sampling:

- `Procedural<T>::sample(ctx)` delegates to `T::from_procedural(kind, ctx)`

This inversion (“T decides how to sample”) is what makes:

- `ProceduralKind::Scalar` usable for `f64` and `f32`
- `ProceduralKind::Vec2` usable for `Vec2`
- and rejects invalid pairings with good errors.

---

### `trait ProcValue`

```
pub trait ProcValue {
  fn from_procedural(kind: &ProceduralKind, ctx: SampleCtx) -> WavyteResult<Self>;
}
```

This is the “bridge” from generic procedural description to a concrete type.

In `anim.rs`, `Anim<T>` requires `T: ProcValue` for this reason.

---

### `enum ProceduralKind`

Serde representation:

- `#[serde(tag = "kind", content = "params")]`

So JSON looks like:

```
{ "kind": "Scalar", "params": ... }
```

Variants:

- `Scalar(ProcScalar)`
- `Vec2 { x: ProcScalar, y: ProcScalar }`

Interpretation:

- `Vec2` is built from two independent scalar signals.

---

### `enum ProcScalar`

Variants:

- `Sine { amp, freq_hz, phase, offset }`
- `Noise1D { amp, freq_hz, offset }`
- `Envelope { attack, decay, sustain, release }`
- `Spring { stiffness, damping, target }`

All of these are evaluated as a function of:

- time (`secs`, derived from frame and fps)
- seed (`ctx.seed`)

---

### `struct Rng64` + `noise01`

`Rng64` is a simple deterministic PRNG:

- Implements SplitMix64 (a common “fast seed mixer”).
- Given the same seed, it produces the same sequence.

`noise01(seed, x)`:

- Builds a `Rng64` seeded by `seed ^ (x * constant)`.
- Returns a stable pseudo-random f64 in [0,1).

Why `noise01` takes `(seed, x)`:

- It allows “random values per lattice point”:
  - sample noise at integer points and interpolate.

---

### `sample_scalar(s, fps, frame, seed) -> f64`

This function is the procedural “DSP core”.

It converts:

- a `ProcScalar` description
- at a given time (frame → secs)
- and a given seed

into a `f64` sample.

#### `Sine`

Formula:

```
offset + amp * sin(TAU * freq_hz * secs + phase)
```

#### `Noise1D`

Interpretation:

- 1D value noise:
  - choose pseudo-random values at integer lattice points
  - linearly interpolate between neighbors

Algorithm:

1. `x = secs * freq_hz`
2. `i0 = floor(x)`
3. `t = x - i0` (fractional)
4. sample random `a` at i0 and `b` at i1
5. interpolate `v = a + (b-a)*t`
6. output `offset + amp*v`

Important:

- random values are mapped from [0,1) → [-1,1) via `*2-1`

#### `Envelope`

This is an ADSR-like envelope in *frames* (not seconds):

- attack: ramp 0 → 1
- decay: ramp 1 → sustain
- release: ramp sustain → 0
- after release: 0

Edge-case handling:

- if a segment duration is 0, that segment is skipped.

ASCII: envelope phases:

```
value
1.0 |      ________
    |     /        \\
    |    /          \\
s   |___/            \\____
0.0 +-----------------------> frame
      A    D     R
```

#### `Spring`

This is a simple critically-damped-like response:

- Not a full physical integrator (no explicit velocity state).
- Instead, uses an exponential term to approach `target`.

Parameters:

- `stiffness` controls how fast the response is.
- `damping` slows it down (in a heuristic way).

Implementation notes:

- clamps `stiffness` and `damping` to non-negative.
- derives `rate = (omega / (1 + d)).max(1e-6)`
- uses:
  - `e = exp(-rate * secs)`
  - `target * (1 - e * (1 + rate*secs))`

---

### `ProcValue` implementations

#### `impl ProcValue for f64`

- Accepts `ProceduralKind::Scalar`
- Rejects `ProceduralKind::Vec2` with an animation error

This prevents accidental type mismatch:

```
Procedural<f64> kind=Vec2 => error
```

#### `impl ProcValue for f32`

- Delegates to `f64` and casts.

#### `impl ProcValue for Vec2`

- Accepts `ProceduralKind::Vec2 { x, y }` and samples each scalar.
- Rejects `Scalar` with an error.

#### `impl ProcValue for Transform2D` and `Rgba8Premul`

Both return “not supported in v0.1”.

Why:

- Supporting procedural transforms/colors is possible, but would expand scope and require clear
  semantics (e.g., how to map scalar signals into multiple fields).

---

### `anim_proc.rs` tests

#### `rng_is_deterministic`

- Constructs two RNGs with same seed, ensures outputs match.

#### `noise_is_bounded_and_deterministic`

Ensures:

- noise changes over time (v0 != v1)
- values are bounded by `offset ± amp` (with slack due to mapping)
- sampling at same frame+seed yields same result

#### `envelope_basic_boundaries`

Ensures:

- at frame 0, envelope starts at 0
- at frame == attack, reaches ~1
- after decay, hits sustain
- after release end, returns to 0

---

<!-- CHUNK 2 END -->


# CHUNK 3 — `anim_ops.rs` + `transitions.rs`

This chunk covers:

- `src/anim_ops.rs`
- `src/transitions.rs`

---

## File: `src/anim_ops.rs`

### Purpose

`anim_ops.rs` is a small “convenience API” layer over `Anim<T>` expression constructors.

Why it exists:

- `Anim::Expr(Expr::Delay { inner: Box::new(...) ... })` is verbose at call sites.
- Builder code and examples benefit from small helpers that read like “operators”.

What it is NOT:

- It does not add new semantics; it just packages `Expr` variants.

ASCII: layers

```
Anim<T> and Expr<T>  (src/anim.rs)
        ^
        |
 convenience wrappers (src/anim_ops.rs)
```

---

### Imports

`use crate::anim::{Anim, Expr, LoopMode};`

This makes the module purely about “wiring existing animation building blocks”.

---

### Helper: `delay(inner, by_frames) -> Anim<T>`

Returns:

```
Anim::Expr(Expr::Delay { inner: Box::new(inner), by: by_frames })
```

Meaning:

- Exposes “delay by N frames” as a single function call.

---

### Helper: `speed(inner, factor) -> Anim<T>`

Returns:

```
Anim::Expr(Expr::Speed { inner: Box::new(inner), factor })
```

Meaning:

- Exposes “play faster/slower” (time scaling).

Important:

- `Expr::sample` will validate factor > 0 at runtime too, but the API keeps the same behavior.

---

### Helper: `reverse(inner, duration_frames) -> Anim<T>`

Returns:

```
Anim::Expr(Expr::Reverse { inner: Box::new(inner), duration: duration_frames })
```

Meaning:

- Exposes “play backwards over a fixed duration”.

---

### Helper: `loop_(inner, period_frames, mode) -> Anim<T>`

Returns:

```
Anim::Expr(Expr::Loop { inner: Box::new(inner), period: period_frames, mode })
```

Note the name `loop_`:

- `loop` is a Rust keyword.

---

### Helper: `mix(a, b, t) -> Anim<T>`

Returns:

```
Anim::Expr(Expr::Mix { a: Box::new(a), b: Box::new(b), t: Box::new(t) })
```

Meaning:

- Exposes “crossfade between two animations” where `t` is itself animatable.

---

### Helper: `sequence(a, a_len, b) -> Anim<f64>`

This is the first “non-trivial” helper.

Goal:

- Provide a “hard switch” from animation `a` to animation `b` at frame `a_len`.

How it is built:

1. Create `b_local = delay(b, a_len)`
   - This remaps time such that when the sequence switches at `a_len`, `b`’s effective time starts
     at 0.
2. Create a step function `t_step`:
   - a keyframed `Anim<f64>` where:
     - value = 0.0 up to `a_len`
     - value = 1.0 starting at `a_len`
   - implemented via `InterpMode::Hold` so it is a true step, not a ramp.
3. Return `mix(a, b_local, t_step)`

ASCII timeline:

```
time ---->

a(t):      a a a a a a a a a a | a a a ...
b(t):      b b b b b b b b b b | b b b ...

sequence:
           a a a a a a a a a a | b b b ...
                             switch at a_len
```

Important subtlety:

- This helper is for `Anim<f64>` specifically (not generic `Anim<T>`).
- That’s because it’s using `t_step` typed as `Anim<f64>` and expects to mix numeric values.

Why this is still useful:

- Many “operator-like” sequences are easiest to express for scalar parameters (opacity, progress).

---

### Helper: `stagger(anims: Vec<(offset, Anim<f64>)>) -> Anim<f64>`

Goal:

- Build a single `Anim<f64>` that “activates” multiple animations at specified offsets.

Algorithm:

1. Sort by offset.
2. If empty:
   - return `Anim::constant(0.0)`
3. Take the first animation:
   - `out = delay(first_anim, first_offset)`
4. For each subsequent `(offset, anim)`:
   - `out = sequence(out, offset, anim)`

Interpretation:

- Earlier animations are active until their switch time.
- At each switch time, the next animation is selected and remapped to start at 0.

ASCII:

```
offsets: (10,a), (40,b), (90,c)

time:    0..9  10..39  40..89  90...
result:   0     a       b      c
```

---

### Tests in `anim_ops.rs`

#### `sequence_switches_at_boundary`

Constructs:

- `a = Anim::constant(1.0)`
- `b` = keyframed hold animation producing 10.0

Then:

- at frame 4 (before switch), `sequence(a,5,b)` samples 1.0
- at frame 5 (switch point), it samples 10.0

This verifies:

- the step function is implemented correctly
- the `delay(b, a_len)` remap aligns `b` to start at the switch boundary

---

## File: `src/transitions.rs`

### Purpose

`transitions.rs` is the “parsing + typing layer” for transitions.

Where transitions fit:

- `model.rs` stores user-authored `TransitionSpec` (string kind + JSON params).
- `transitions.rs` converts that into a strongly typed `TransitionKind`.
- `eval.rs` uses this to produce `ResolvedTransition` instances.
- `compile.rs` uses those to emit `CompositePass` operations.

ASCII: transition flow:

```
TransitionSpec (string + JSON)  [model.rs]
           |
           v
TransitionKind (typed enum)     [transitions.rs]
           |
           v
ResolvedTransition (with progress, etc.) [eval.rs]
           |
           v
CompositePass ops (crossfade/wipe) [compile.rs]
```

---

### Imports

```
use crate::{
  error::{WavyteError, WavyteResult},
  model::TransitionSpec,
};
```

This is a classic “model → typed representation” module:

- errors are validation errors when kind/params are invalid
- spec is read-only input

---

### `enum WipeDir`

Directions:

- `LeftToRight`
- `RightToLeft`
- `TopToBottom`
- `BottomToTop`

This stays small on purpose:

- all wipe variants can be expressed by direction + soft edge.

---

### `enum TransitionKind`

Currently supported:

- `Crossfade`
- `Wipe { dir, soft_edge }`

`soft_edge`:

- f32 in [0,1]
- describes the blend band around the wipe boundary

---

### `parse_transition_kind_params(kind: &str, params: &serde_json::Value)`

This is the main parser.

Step-by-step:

1. normalize kind:
   - trim
   - lowercase
2. kind must be non-empty
3. dispatch:
   - `"crossfade"` => `TransitionKind::Crossfade`
   - `"wipe"` => parse params object and return `TransitionKind::Wipe`
   - otherwise => validation error

---

### Wipe parsing details

Input params:

- If `params` is JSON null:
  - treat as no params (use defaults)
- Else:
  - require it is a JSON object

#### `dir`

Default:

- Left-to-right

Aliases:

- `"left_to_right"`, `"lefttoright"`, `"ltr"`
- `"right_to_left"`, `"righttoleft"`, `"rtl"`
- `"top_to_bottom"`, `"toptobottom"`, `"ttb"`
- `"bottom_to_top"`, `"bottomtotop"`, `"btt"`

Unknown strings produce:

- `WavyteError::validation("unknown wipe.dir '...'" )`

Why accept aliases:

- JSON authors often prefer shorthand.
- Backwards compatibility is easier if we accept a few common forms.

#### `soft_edge`

Default:

- 0.0

Parsing:

- reads `as_f64` from JSON
- converts to f32
- requires finite
- clamps to [0,1]

Reason for clamp:

- we want robust user input handling
- downstream math assumes [0,1]

---

### `parse_transition(spec: &TransitionSpec)`

Just delegates:

```
parse_transition_kind_params(&spec.kind, &spec.params)
```

This is primarily for ergonomic call sites.

---

### Tests in `transitions.rs`

#### `wipe_dir_parses_aliases`

Given:

- kind="wipe"
- params={"dir":"ttb","soft_edge":0.1}

Expect:

- `Wipe { dir: TopToBottom, soft_edge: 0.1 }`

#### `wipe_soft_edge_is_clamped`

Given:

- soft_edge=-5.0

Expect:

- clamped to 0.0

Also implicitly verifies:

- default `dir` = LeftToRight when missing

---

<!-- CHUNK 3 END -->


# CHUNK 4 — `model.rs` + `fx.rs`

This chunk covers:

- `src/model.rs` (the data model + validation)
- `src/fx.rs` (effects parsing + normalization)

These are foundational because:

- Most of the pipeline is “just transforming the model into IR and then executing it”.
- Good validation is what keeps later stages simpler and less error-prone.

---

## File: `src/model.rs`

### Purpose

`model.rs` defines the **composition data model**: the “source of truth” timeline description that
evaluation and compilation operate on.

Guiding principle:

- The `Composition` itself is intended to be “pure data”:
  - Serializable (Serde)
  - Validatable
  - Not bound to any particular backend

ASCII: model vs pipeline

```
   (model.rs)                    (eval.rs)          (compile.rs)        (render_*.rs)
Composition --------frame------> EvaluatedGraph ---> RenderPlan -------> pixels
```

---

### Imports

`model.rs` depends on:

- `std::collections::BTreeMap`
  - for stable ordering of assets by key
- `crate::anim::Anim`
  - because clip properties are animated values
- `crate::anim_ease::Ease`
  - because transitions include an ease curve
- `crate::core::{Canvas, Fps, FrameIndex, FrameRange, Transform2D}`
  - core primitives
- `crate::error::{WavyteError, WavyteResult}`
  - validation / result type

Why `BTreeMap` and not `HashMap` for `assets`:

- A `BTreeMap` is ordered by key, which makes:
  - JSON serialization deterministic (stable output)
  - evaluation/compile debug output more stable
  - tests easier (snapshots don’t reorder randomly)

---

### `struct Composition`

Fields:

- `fps: Fps`
  - global timebase (rational)
- `canvas: Canvas`
  - output resolution
- `duration: FrameIndex`
  - total frame count in the composition
- `assets: BTreeMap<String, Asset>`
  - stable key → asset definition
- `tracks: Vec<Track>`
  - ordered list of tracks; later evaluation will determine painter order
- `seed: u64`
  - global determinism seed for procedural animation and any deterministic randomness

Interpretation:

```
Composition is the entire project file.
  - It does not store “derived caches”.
  - It does not store “prepared assets”.
  - It is safe to clone and serialize.
```

---

### `struct Track`

Fields:

- `name: String`
- `z_base: i32`
  - baseline Z offset applied to clips in this track
- `clips: Vec<Clip>`
  - ordered list of clips inside the track

Meaning:

- A track is a container for clips that share a base layer ordering.

ASCII:

```
Track(z_base=100)
  Clip z_offset=0   => z=100
  Clip z_offset=10  => z=110
```

---

### `struct Clip`

Fields:

- `id: String`
  - human readable identifier
- `asset: String`
  - key into `Composition.assets`
- `range: FrameRange`
  - when the clip is active (`[start, end)`)
- `props: ClipProps`
  - transform/opacity/blend
- `z_offset: i32`
  - per-clip Z ordering relative to track’s `z_base`
- `effects: Vec<EffectInstance>`
  - list of effect “specs” (string kind + JSON params)
- `transition_in: Option<TransitionSpec>`
- `transition_out: Option<TransitionSpec>`

Interpretation:

```
Clip = “place asset X on the timeline from start..end, with these properties/effects”.
```

---

### `struct ClipProps`

Fields:

- `transform: Anim<Transform2D>`
  - 2D transform animated per-frame
- `opacity: Anim<f64>`
  - animated opacity factor
- `blend: BlendMode`
  - blend mode (v0.1 only Normal)

Note:

- Opacity is `f64` (animation sampling uses f64 heavily).
- In eval, opacity is clamped to [0,1].

---

### `enum BlendMode`

Currently:

- `Normal` (source over destination, premultiplied alpha)

Why keep an enum if only one variant:

- Future-proofing: design establishes the place where blending rules live.
- Avoids rewiring JSON formats later.

---

### `enum Asset`

Variants:

- `Text(TextAsset)`
- `Svg(SvgAsset)`
- `Path(PathAsset)`
- `Image(ImageAsset)`
- `Video(VideoAsset)`
- `Audio(AudioAsset)`

Important:

- The model includes asset kinds not rendered in v0.1 (video/audio).
- `Composition::validate()` still validates their `source` is a sane relative path.

ASCII: asset kinds

```
Renderable v0.1:  Path, Image, Svg, Text
Model-only v0.1:  Video, Audio
```

---

### Asset structs

#### `TextAsset`

Fields:

- `text: String`
- `font_source: String` (relative file path)
- `size_px: f32`
- `max_width_px: Option<f32>`
  - when present, triggers line breaking/alignment in Parley
- `color_rgba8: [u8; 4]`

Default text color:

- via `default_text_color_rgba8()` returning `[255,255,255,255]`

Serde behavior:

- `max_width_px` is omitted if None.
- `color_rgba8` uses a default if field is missing.

#### `SvgAsset`, `ImageAsset`, `VideoAsset`, `AudioAsset`

All have:

- `source: String` (a relative path)

#### `PathAsset`

Fields:

- `svg_path_d: String`

Meaning:

- The SVG “d” attribute content for a single path.
- v0.1 uses this as a simple vector primitive without external IO.

---

### `struct EffectInstance`

Fields:

- `kind: String`
- `params: serde_json::Value`

Meaning:

- The model stores effects as data-driven “plugin-like” specs.
- Parsing and typing happens in `src/fx.rs`.

Serde behavior:

- `params` omitted if null.

---

### `struct TransitionSpec`

Fields:

- `kind: String`
- `duration_frames: u64`
- `ease: Ease`
- `params: serde_json::Value`

Meaning:

- Like effects, transitions start as “data specs”.
- `src/transitions.rs` parses them into typed `TransitionKind`.
- `eval.rs` converts into “resolved transitions” with progress.

---

## `Composition::validate()`

This is the single most important defensive layer in v0.1.

Validation happens in multiple passes:

### Pass 1: global fields

- `fps.num` and `fps.den` must be non-zero
  - (Note: `Fps::new()` also enforces this, but JSON could bypass constructors.)
- canvas width/height > 0
- duration > 0 frames

### Pass 2: track/clip validation

For each clip:

1. referenced asset must exist in `assets` map
2. clip range must be valid (start <= end)
3. clip range must be within composition duration (end <= duration)
4. validate animations:
   - `clip.props.opacity.validate()`
   - `clip.props.transform.validate()`
5. validate transitions (if present):
   - `transition_in.validate()`
   - `transition_out.validate()`

Why validate `Anim<T>` here:

- It catches broken keyframe ordering or invalid expression parameters early.

### Pass 3: asset validation

For each `(key, asset)`:

1. key must be non-empty after trim
2. asset-specific rules:
   - Text:
     - `text` non-empty
     - `font_source` is a valid relative path
     - `size_px` finite and > 0
     - if max_width set: finite and > 0
   - Svg/Image/Video/Audio:
     - `source` is a valid relative path
   - Path:
     - `svg_path_d` must be non-empty

Crucial detail:

- This validation checks *path form* but does **not** check the file exists.
- Existence is handled later by `AssetCache` during rendering.

This is intentional:

- You can validate compositions in environments without assets present.
- It decouples “structure is valid” from “IO is available”.

---

### Helper: `validate_rel_source(source, field)`

Rules:

- must be non-empty after trim
- normalize slashes: `\` → `/`
- must not start with `/` (no absolute paths)
- must not contain `..` path segments

Why this is strict:

- Prevents directory traversal.
- Forces compositions to be portable across machines.
- Makes asset IDs stable (later we normalize similarly in `FsAssetCache`).

---

### `TransitionSpec::validate()`

Rules:

- kind non-empty
- duration_frames > 0
- params is either null or an object

Why validate params type here:

- Makes parsing predictable for `transitions.rs`
- Avoids weird schema drift

---

### `model.rs` tests

#### `basic_comp()`

Builds a minimal but representative composition:

- one `TextAsset`
- one clip that references it
- one effect instance (noop)
- one transition spec (crossfade)

Even though “noop” effect is not special-cased in fx parsing, it’s a good smoke structure.

#### `json_roundtrip`

Ensures:

- Composition is serde-serializable and deserializable.
- Some basic fields survive roundtrip.

#### `validate_rejects_missing_asset`

Ensures:

- A clip referencing a missing asset key is rejected.

#### `validate_rejects_out_of_bounds_range`

Ensures:

- Clip ranges cannot exceed composition duration.

#### `validate_rejects_bad_fps`

Ensures:

- JSON that sets `fps.den=0` is rejected (even though `Fps::new` would prevent it).

---

## File: `src/fx.rs`

### Purpose

`fx.rs` is the “effect system” in v0.1.

It provides two layers:

1. Parsing:
   - `EffectInstance` (string + JSON) → typed `Effect`
2. Normalization:
   - fold a list of `Effect` values into a structured `FxPipeline`:
     - “inline” effects that can be applied at compile time
     - “pass” effects that require offscreen rendering (e.g. blur)

ASCII: effect flow

```
EffectInstance (model)  --> parse_effect() --> Effect (typed)
                                        |
                                        v
                             normalize_effects()
                                        |
                                        v
                               FxPipeline { inline, passes }
```

---

### `enum Effect`

Variants:

- `OpacityMul { value: f32 }`
  - multiply clip opacity
- `TransformPost { value: Affine }`
  - post-multiply clip transform
- `Blur { radius_px: u32, sigma: f32 }`
  - pass effect; becomes an offscreen render + blur kernel

Why `TransformPost` is an `Affine`:

- It composes naturally with `Transform2D::to_affine()` in evaluation/compile.

---

### `struct InlineFx`

Fields:

- `opacity_mul: f32`
- `transform_post: Affine`

Meaning:

- Inline effects can be folded into:
  - `DrawOp.opacity`
  - `DrawOp.transform`
  - without requiring additional surfaces/passes

Default:

- opacity_mul = 1.0
- transform_post = identity

---

### `enum PassFx`

Currently:

- `Blur { radius_px, sigma }`

Meaning:

- Pass effects are executed by the compiler by emitting:
  - an offscreen pass to render the content
  - then a pass to apply the effect
  - then compositing results back

---

### `struct FxPipeline`

Fields:

- `inline: InlineFx`
- `passes: Vec<PassFx>`

Default:

- inline defaults
- passes empty

This is the normalized representation that downstream compile uses.

---

### `parse_effect(inst: &EffectInstance) -> WavyteResult<Effect>`

Parsing steps:

1. normalize `kind`:
   - trim
   - lowercase
2. kind must be non-empty
3. dispatch by kind:

#### OpacityMul

Accepted kind aliases:

- `opacitymul`, `opacity_mul`, `opacity-mul`

Params:

- `value` (required), parsed as f32

Validation:

- finite
- >= 0 (negative would invert opacity, not supported)

#### TransformPost

Accepted aliases:

- `transformpost`, `transform_post`, `transform-post`

Params:

- parsed by `parse_affine(params)`

#### Blur

Kind:

- `blur`

Params:

- `radius_px` (required u32)
- `sigma` (optional f32)
  - if absent, defaults to `radius_px / 2.0`

Validation:

- `radius_px <= 256` in v0.1 (arbitrary safety cap)
- sigma must be finite and > 0

Unknown kind:

- validation error `"unknown effect kind '...'"`.

---

### `normalize_effects(effects: &[Effect]) -> FxPipeline`

Normalization folds effects with the following rules:

1. Start with defaults:
   - inline.opacity_mul = 1.0
   - inline.transform_post = identity
   - passes empty
2. For each effect:
   - OpacityMul:
     - multiply into inline.opacity_mul (fold)
   - TransformPost:
     - post-multiply into inline.transform_post (fold)
   - Blur:
     - if radius_px == 0 => drop (no-op)
     - else push PassFx::Blur
3. Post-check:
   - if opacity_mul becomes NaN/Inf or negative, clamp to 0
4. If everything is identity/no passes:
   - return default pipeline (canonical no-op)
   - else return `FxPipeline { inline, passes }`

Why folding is good:

- Multiple opacity multipliers can be collapsed into one.
- Multiple transform post-muls can be collapsed into one affine matrix.
- This reduces downstream work.

---

### Helper parsers: `get_u32`, `get_f32`

Both:

- require the key exists
- enforce numeric type
- enforce finiteness
- provide good error messages

This keeps `parse_effect` readable.

---

### `parse_affine(params)`

Two supported param shapes:

1. Direct `affine: [a,b,c,d,e,f]` array (length 6)
2. Structured fallback:
   - translate: [x,y]
   - rotation_rad or rotate_deg
   - scale: [sx,sy]

Return:

- `t * rot * scale`

Interpretation:

- The structured fallback is intentionally limited (no skew).
- The full `affine` array exists for power users.

---

### `fx.rs` tests

#### `parse_opacity_mul`

Confirms:

- alias `opacity_mul` parses correctly
- returns `Effect::OpacityMul { value: 0.5 }`

#### `normalize_folds_opacity_and_drops_noop_blur`

Given effects:

- opacity_mul 0.5
- opacity_mul 0.25
- blur radius 0 (no-op)

Expect:

- folded opacity_mul = 0.125
- no passes (blur dropped)

This verifies two key behaviors:

- folding is multiplicative
- radius 0 blur is treated as no-op

---

<!-- CHUNK 4 END -->


# CHUNK 5 — `eval.rs` (evaluation stage)

This chunk covers:

- `src/eval.rs`

This is pipeline stage (1):

```
Composition + FrameIndex  ->  EvaluatedGraph
```

Where “evaluated” means:

- animations sampled at the current frame
- visibility filtered by clip range
- painter order resolved (stable ordering)
- transitions turned into “progress in [0,1] at this frame”

---

## File: `src/eval.rs`

### Purpose

`eval.rs` is the bridge between:

- pure timeline data (`model.rs`)
- and the render compiler (`compile.rs`)

Its output (`EvaluatedGraph`) is:

- render-backend agnostic
- stable/deterministic for a given input
- suitable for testing and snapshotting

ASCII: evaluation boundaries

```
INPUT:  Composition (tracks/clips/assets, Anim<T>, effects, transitions)
OUTPUT: EvaluatedGraph (flat list of EvaluatedClipNode)
```

---

### Imports

`eval.rs` uses:

- `anim::SampleCtx` (to sample animations)
- `core::{FrameIndex, FrameRange}` (time)
- `error::{WavyteError, WavyteResult}` (errors)
- `model::{BlendMode, Clip, Composition, EffectInstance, TransitionSpec}` (model pieces)

---

### Output structs

#### `struct EvaluatedGraph`

Fields:

- `frame: FrameIndex` (the evaluated frame)
- `nodes: Vec<EvaluatedClipNode>` (painter-sorted nodes)

This is a “flat scene graph” for one frame.

#### `struct EvaluatedClipNode`

Fields:

- `clip_id: String` (copy of model id)
- `asset: String` (asset key to look up in composition / asset cache)
- `z: i32` (fully resolved z = track.z_base + clip.z_offset)
- `transform: kurbo::Affine` (fully sampled transform)
- `opacity: f64` (sampled and clamped to [0,1])
- `blend: BlendMode` (currently only Normal)
- `effects: Vec<ResolvedEffect>` (still data-driven; parsed later)
- `transition_in: Option<ResolvedTransition>`
- `transition_out: Option<ResolvedTransition>`

Important:

- `EvaluatedClipNode` does not contain actual `Asset` objects; it holds the asset key.
- This keeps evaluation pure and avoids IO (asset loading is deferred).

#### `struct ResolvedEffect`

Fields:

- `kind: String`
- `params: serde_json::Value`

Why this exists:

- Evaluation currently just validates effect kinds are non-empty.
- Detailed parsing happens in `fx.rs` during compilation.

#### `struct ResolvedTransition`

Fields:

- `kind: String`
- `progress: f64` (0..1)
- `params: serde_json::Value`

Note:

- `progress` is pre-eased using the transition’s `Ease` curve.

---

### `struct Evaluator` + `eval_frame`

`Evaluator` is a namespace struct: it carries no fields, just behavior.

#### `Evaluator::eval_frame(comp, frame)`

Algorithm overview:

1. Validate the composition (`comp.validate()`).
   - This makes eval safe even if called independently of the pipeline helpers.
2. Ensure `frame < comp.duration`.
3. Iterate tracks and clips:
   - if clip is not visible at this frame, skip
   - else produce an `EvaluatedClipNode` via `eval_clip`
4. Sort nodes into painter order using an explicit sort key.
5. Return `EvaluatedGraph { frame, nodes }`.

The sort key used is:

```
(z, track_index, clip.range.start, clip_id)
```

Why this sort key:

- `z` gives intended “layering” control.
- `track_index` makes ordering stable when two clips share the same z.
- `clip.range.start` makes earlier-starting clips sort earlier when ties remain.
- `clip_id` ensures deterministic ordering even if everything else ties.

ASCII: deterministic painter order

```
node A: z=10, track=0, start=0, id="a"
node B: z=10, track=0, start=0, id="b"

=> A before B always
```

---

### `eval_clip(comp, clip, frame, track_z_base)`

This is the per-clip sampler.

Key steps:

1. Compute `clip_local = frame - clip.range.start`
2. Compute a per-clip seed:
   - `seed = stable_hash64(comp.seed, &clip.id)`
3. Build `SampleCtx { frame, fps: comp.fps, clip_local, seed }`
4. Sample and clamp:
   - `opacity = clip.props.opacity.sample(ctx)?.clamp(0,1)`
   - `transform = clip.props.transform.sample(ctx)?.to_affine()`
5. Resolve effects:
   - `effects = clip.effects.iter().map(resolve_effect)`
6. Resolve transitions:
   - `transition_in = resolve_transition_in(...)`
   - `transition_out = resolve_transition_out(...)`
7. Construct `EvaluatedClipNode`

Note on transform:

- `Transform2D` (model space) is converted into `kurbo::Affine` (render space).

---

### `resolve_effect(e: &EffectInstance)`

Behavior:

- Rejects empty/whitespace-only kinds as an evaluation error.
- Otherwise, “passes through” kind and params unchanged.

Why evaluation error and not validation error:

- The composition has already been validated structurally.
- This is “runtime stage logic” (although it’s still about user data).
- In practice either category would be reasonable; v0.1 uses Evaluation here.

---

### Transition resolution

The model has:

- `transition_in`: logically at clip start
- `transition_out`: logically at clip end

But transitions are evaluated within a window.

#### `resolve_transition_in`

Calls:

```
resolve_transition_window(spec, frame, clip.range, clip.range.start, In)
```

#### `resolve_transition_out`

Calls:

```
resolve_transition_window(spec, frame, clip.range, clip.range.end, Out)
```

#### `resolve_transition_window(...)`

Key semantics:

- If duration is 0 → no transition.
- If clip length is 0 → no transition.
- Effective transition duration is `min(spec.duration_frames, clip_len)`.

Window computation:

- For IN:
  - window = `[clip.start, clip.start + dur)`
- For OUT:
  - window = `[clip.end - dur, clip.end)`

If current frame is outside window:

- return None

Progress computation:

- denom = `dur - 1` (so last frame hits 1.0)
- If denom == 0:
  - progress = 1.0
- Else:
  - `t = (frame - window_start) / denom`
  - `progress = spec.ease.apply(t).clamp(0,1)`

Why denom is `dur-1`:

- If dur is 3 frames, we want:
  - first frame progress 0.0
  - middle frame progress 0.5
  - last frame progress 1.0

ASCII: `dur=3` mapping

```
frames: 0   1   2
offset: 0   1   2
denom:  2
t:      0   0.5 1
```

---

### `stable_hash64(seed, s)`

This function implements seeded FNV-1a 64-bit hashing.

Purpose:

- Derive per-clip deterministic seeds from:
  - global composition seed
  - clip id string

This keeps procedural motion stable even when:

- clip order changes
- new clips are added

Because each clip’s seed depends only on its id and the composition seed.

---

### `eval.rs` tests

These tests all use `basic_comp(opacity, tr_in, tr_out)` which creates:

- a composition with a single text asset + one clip
- clip range `[5, 15)`

#### `visibility_respects_frame_range`

Asserts:

- frame 4: invisible
- frames 5..14: visible
- frame 15: invisible (end is exclusive)

#### `opacity_is_clamped`

Uses an opacity animation that returns 2.0.

Asserts:

- evaluated opacity is 1.0 (clamped).

#### `transition_progress_boundaries`

Creates a transition with:

- duration_frames=3
- linear ease

Asserts:

- transition_in:
  - at clip start (frame 5), progress is 0.0
  - at last in-transition frame (frame 7), progress is 1.0
- transition_out:
  - starts at end-dur (frame 12), progress is 0.0
  - last frame (frame 14), progress is 1.0

This effectively regression-tests the denom=`dur-1` choice.

---

<!-- CHUNK 5 END -->


# CHUNK 6 — `assets.rs` + `assets_decode.rs` + `svg_raster.rs`

This chunk covers:

- `src/assets.rs`
- `src/assets_decode.rs`
- `src/svg_raster.rs`

These modules define the “asset boundary” of Wavyte:

- the renderer/compiler can only deal with **prepared** assets
- IO and decoding are centralized in an `AssetCache` implementation

ASCII: “prepared asset” boundary

```
filesystem bytes  --->  FsAssetCache  --->  PreparedAsset  ---> renderer draws
    (IO)                 (decode)            (pure)            (no IO)
```

---

## File: `src/assets.rs`

### Purpose

`assets.rs` defines:

1. The prepared asset types (`PreparedImage`, `PreparedSvg`, `PreparedText`, `PreparedAsset`)
2. The `AssetCache` trait (IO boundary)
3. `FsAssetCache` (filesystem-backed implementation)
4. Stable `AssetId` derivation + asset key normalization
5. Text layout engine wrapper (`TextLayoutEngine`) and font verification metadata
6. SVG font handling policy (system fonts + local fonts) and a permissive font resolver

This module is intentionally large because it is:

- the only place in v0.1 where filesystem reads occur (outside of the CLI wrapper)

---

### Prepared asset types

#### `PreparedImage`

Fields:

- `width: u32`, `height: u32`
- `rgba8_premul: Arc<Vec<u8>>`

Key contract:

- Always premultiplied RGBA8.
- Stored in `Arc<Vec<u8>>` so clones are cheap and backend caches can share memory.

Where it is produced:

- `assets_decode::decode_image()`

Where it is consumed:

- `render_cpu.rs` and `render_vello.rs` draw it as an image draw op.

#### `PreparedSvg`

Fields:

- `tree: Arc<usvg::Tree>`

Key contract:

- Stores the parsed SVG tree (vector semantics).
- Rendering in v0.1 is rasterized later (via `resvg`), but we keep the tree for:
  - caching
  - extracting intrinsic size
  - future vector pipelines

#### `TextBrushRgba8`

This is a small “brush” type used by Parley.

Fields:

- `r,g,b,a` as u8

Note:

- It’s intentionally separate from `Rgba8Premul`, because Parley wants a “Brush” type and text fill
  is conceptually “straight color” at the brush level.

#### `PreparedText`

Fields:

- `layout: Arc<parley::Layout<TextBrushRgba8>>`
- `font_bytes: Arc<Vec<u8>>`
- `font_family: String`

Why store `font_bytes`:

- Renderers must not do IO.
- If glyph rasterization needs font data, it must be carried with the prepared asset.

Why store `font_family`:

- This was added as a “verification hook”:
  - users can confirm which family name was registered from their provided bytes.

Custom `Debug` impl:

- Avoids dumping the entire layout; prints:
  - layout pointer
  - byte length
  - family name

This keeps logs readable.

#### `enum PreparedAsset`

Variants:

- `Image(PreparedImage)`
- `Svg(PreparedSvg)`
- `Text(PreparedText)`

This is the return type of `AssetCache::get_or_load`.

---

### Stable IDs: `AssetId` and `AssetKey`

#### `struct AssetId(u64)`

Meaning:

- A stable identifier for a prepared asset (within a given composition/assets map).

In v0.1:

- It is derived from:
  - a kind tag (I/S/T)
  - a normalized asset path
  - plus “parameters” that affect preparation

#### `struct AssetKey`

Fields:

- `norm_path: String`
- `params: Vec<(String,String)>`

Important:

- params are sorted in `AssetKey::new()` so ordering does not affect the hash.

Why params exist:

- Text preparation depends on:
  - the string content
  - size
  - color
  - optional max width
- So two different `TextAsset` values that point at the same font file must still produce different
  `AssetId`s.

---

### `trait AssetCache`

The IO boundary has three methods:

- `id_for(asset) -> AssetId`
  - registers the asset and returns the stable id
- `get_or_load(asset) -> PreparedAsset`
  - main entry point: memoized preparation
- `get_or_load_by_id(id) -> PreparedAsset`
  - used by render plans that refer to assets by id

Key design constraint:

- Renderers can only call these methods.
- They cannot read disk directly.

---

### `struct FsAssetCache`

Fields:

- `root: PathBuf` (filesystem root)
- `keys_by_id: HashMap<AssetId, AssetKey>`
- `asset_by_id: HashMap<AssetId, model::Asset>`
- `prepared: HashMap<AssetId, PreparedAsset>`
- `decode_counts: HashMap<AssetId, u32>` (test helper)
- `text_engine: TextLayoutEngine`

Interpretation:

```
FsAssetCache is a memoization layer:
  Asset (model) -> AssetKey -> AssetId -> PreparedAsset
```

Why both `keys_by_id` and `asset_by_id` exist:

- `get_or_load_by_id` needs to recover the original `model::Asset` definition.
- `id_for` stores that association when the id is computed.

---

### `FsAssetCache::key_for(asset) -> (kind_tag, AssetKey)`

This function defines what makes an asset “distinct”.

Kind tags:

- Image => `b'I'`
- Svg   => `b'S'`
- Text  => `b'T'`

For image/svg:

- params empty
- key uses normalized path

For text:

- key path is `font_source`
- params include:
  - `text`
  - `size_px_bits` (bitwise float stability)
  - `color_rgba8`
  - optional `max_width_px_bits`

Float bit encoding detail:

- Uses `f32::to_bits()` and formats as hex.
- This makes different NaN payloads or subtle float differences distinct (v0.1 chooses “bitwise
  identity” semantics).

Unsupported kinds for FsAssetCache in v0.1:

- Video/Audio/Path return a validation error (“not yet supported by FsAssetCache in phase 3”).

---

### Path normalization: `normalize_rel_path(source)`

Rules (same spirit as `model::validate_rel_source`, but returns the normalized path string):

- convert `\` to `/`
- reject leading `/`
- reject empty
- drop `.` and empty segments
- reject `..`
- join segments with `/`

Why do this twice (model + assets):

- Model validation ensures user composition is well-formed.
- FsAssetCache normalization ensures:
  - stable asset keying
  - OS-agnostic path keys
  - no traversal at IO boundary

---

### Hashing: `id_for_key(kind_tag, key) -> AssetId`

Implements a deterministic FNV-1a 64-bit hash:

- writes kind tag
- writes norm_path
- separator 0 byte
- writes each param key/value separated by 0 bytes

`Fnv1a64` helper:

- simple implementation with the standard offset basis and prime.

Test `asset_id_stability_same_input` locks in a specific expected hash value:

- This is a deliberate “stability contract” test:
  - if you change hashing inputs/algorithm, this test will break and force a conscious decision.

---

### Reading bytes: `read_bytes(norm_path)`

Implementation:

- joins `root` + `norm_path`
- reads file to Vec<u8>
- wraps IO errors with context

Important:

- `root` is a `PathBuf` and `norm_path` uses `/`.
- `Path::new(norm_path)` works cross-platform; on Windows `/` paths still parse as relative path
  components.

---

### SVG parsing path: `parse_svg_with_options(norm_path, bytes)`

This is the uplift-A SVG correctness path.

Key steps:

1. Compute `abs = root.join(norm_path)`
2. Set `resources_dir = abs.parent()`
3. Build font database:
   - `build_svg_fontdb(resources_dir)`
4. Install a custom `font_resolver`:
   - `make_svg_font_resolver()`
5. Construct `usvg::Options { resources_dir, fontdb, font_resolver, ..Default }`
6. Parse:
   - `usvg::Tree::from_data(bytes, &opts)`

Why do this at parse time:

- `usvg` decides whether to include `<text>` nodes while parsing/layouting text.
- If font resolution fails, text nodes can be dropped.

---

### SVG font database policy: `build_svg_fontdb(resources_dir)`

Policy chosen (per your decision):

- system fonts ON
- project fonts ON
- tests rely on vendored font fixtures

Implementation:

1. `Database::new()`
2. `db.load_system_fonts()`
   - best-effort directory scan (platform-specific)
3. Load local fonts from:
   - `<root>/fonts` (if exists)
   - `<root>/assets` (common in this repo)
4. Also load fonts near the SVG:
   - `resources_dir` itself
   - `resources_dir/fonts`

Font scan is non-recursive and extension-filtered:

- `.ttf`, `.otf`, `.ttc`

Load errors are ignored:

- partial success is better than aborting SVG parsing.

---

### SVG font resolver: `make_svg_font_resolver()`

This was added to fix “SVG `<text>` silently disappears when no family matches”.

Mechanism:

- Provide a custom `FontResolver` with a `select_font` callback.

Strategy:

1. Convert `usvg::FontFamily` list into `fontdb::Family` query list.
2. Add permissive fallbacks:
   - SansSerif, Serif, Monospace
3. Query the fontdb.
4. If still no match:
   - fall back to “any face at all” (`fontdb.faces().next()`)

This trades typography fidelity for correctness:

- Text is visible even if it uses the “wrong” font.
- Users can fix font families later; the renderer doesn’t just drop the node.

---

### `impl AssetCache for FsAssetCache`

#### `id_for(asset)`

Behavior:

- derive (kind,key)
- derive id
- store:
  - keys_by_id[id] = key
  - asset_by_id[id] = asset clone
- return id

#### `get_or_load(asset)`

Behavior:

- derive (kind,key) and id (same as `id_for`)
- check memo table:
  - if prepared exists, return clone
- else load bytes and decode based on asset kind:
  - Image:
    - read bytes
    - `assets_decode::decode_image`
  - Svg:
    - read bytes
    - `parse_svg_with_options` (not `assets_decode::parse_svg`)
  - Text:
    - read font bytes
    - layout with `TextLayoutEngine::layout_plain`
    - capture family name for verification
- record decode count
- store prepared
- return prepared

Why Text uses `TextLayoutEngine`:

- Centralizes Parley font registration and layout building.

#### `get_or_load_by_id(id)`

Behavior:

- look up the registered `model::Asset` in `asset_by_id`
- error if missing (evaluation error)
- call `get_or_load(asset)`

Why `id_for` matters:

- Many code paths compute ids up-front so render plans can refer to ids.
- If you call `get_or_load_by_id` for an id you never registered, you get an explicit error.

---

### `TextLayoutEngine`

Fields:

- `font_ctx: parley::FontContext`
- `layout_ctx: parley::LayoutContext<TextBrushRgba8>`
- `last_family_name: Option<String>` (verification hook)

#### `layout_plain(text, font_bytes, size_px, brush, max_width)`

Steps:

1. Validate `size_px` is finite and > 0.
2. Register font bytes into Parley’s font collection:
   - `register_fonts(Blob::from(font_bytes.to_vec()), None)`
3. Extract the first returned family id.
4. Read the family name and store it into `last_family_name`.
5. Build a ranged layout builder:
   - set FontStack to the family name
   - set FontSize
   - set Brush
6. Build layout and apply line breaking:
   - if max_width set:
     - `break_all_lines(Some(w))`
     - `align(Some(w), Start, ...)`
   - else:
     - `break_all_lines(None)`

This produces a `parley::Layout<TextBrushRgba8>` suitable for backends.

---

### `assets.rs` tests (high-level intent)

- path normalization tests ensure portability and traversal safety.
- asset id stability test ensures hashing contract.
- cache decode count test ensures memoization.
- `parley_brush_type_is_valid` ensures our brush type implements Parley’s Brush trait.
- `text_layout_smoke_with_local_font_if_present` is a “dev machine smoke” test that only runs when
  `assets/PlayfairDisplay.ttf` exists (it early-returns if missing).

---

## File: `src/assets_decode.rs`

### Purpose

`assets_decode.rs` is the pure “decode bytes into prepared asset structs” helper module.

It is intentionally small:

- Image decode: uses the `image` crate
- SVG parse: uses `usvg`
- It avoids filesystem IO (bytes in, prepared out)

Cycle note:

- `assets_decode.rs` returns types defined in `assets.rs` (`PreparedImage`, `PreparedSvg`).

### `decode_image(bytes) -> PreparedImage`

Steps:

1. `image::load_from_memory(bytes)` → dynamic image
2. Convert to RGBA8
3. Premultiply alpha in-place (`premultiply_rgba8_in_place`)
4. Return `PreparedImage` with `Arc<Vec<u8>>`

### `parse_svg(bytes) -> PreparedSvg`

This is a “default options” parser used primarily for in-memory SVG tests.

Important:

- It uses `usvg::Options::default()`.
- It does NOT install the `FsAssetCache` fontdb policy and font resolver.
- Therefore, it is not the canonical path for filesystem SVG assets anymore.

### `premultiply_rgba8_in_place`

Per pixel:

- if alpha=0 → set RGB to 0
- else:
  - `rgb = (rgb * a + 127) / 255`

Same premultiply formula as `core::Rgba8Premul::from_straight_rgba`.

### Tests

- `decode_image_png_dimensions_and_premul`:
  - builds a 1×1 png in memory and checks premultiply math
- `decode_svg_parse_ok_and_err`:
  - parses a valid minimal svg and ensures invalid bytes error

---

## File: `src/svg_raster.rs`

### Purpose

`svg_raster.rs` is the shared “SVG rasterization policy” module used by both CPU and GPU backends.

It exists so that:

- both backends produce the same SVG visual output
- rasterization size can be “scale-aware” to avoid blurry upscaling
- caching can key on `(AssetId, width, height)`

### `SvgRasterKey`

Fields:

- `asset: AssetId`
- `width: u32`
- `height: u32`

Used as a hash map key in:

- `render_cpu.rs`
- `render_vello.rs`

### `svg_raster_params(tree, transform) -> (w, h, transform_adjust)`

Inputs:

- SVG intrinsic size from `tree.size()`
- final draw transform (`kurbo::Affine` aka `crate::core::Affine`)

Outputs:

- raster width/height (in pixels)
- a modified transform to apply when drawing the rasterized image

Key steps:

1. Read intrinsic width/height (`base_w`, `base_h`) and ceil to u32.
2. Estimate scale from affine coefficients:
   - `sx = hypot(a,b)`
   - `sy = hypot(c,d)`
3. Compute raster size:
   - `w = ceil(base_w * sx)`
   - `h = ceil(base_h * sy)`
4. Reject pathological raster sizes (> 16384).
5. Compute `transform_adjust = transform * scale(1/sx, 1/sy)`

Interpretation:

- We bake the scale into the rasterization itself (higher-res pixmap).
- Then we remove that scale from the draw transform so the image lands in the same place.

ASCII:

```
original: draw SVG with scale(4x)

new policy:
  - rasterize SVG into 4x larger pixmap
  - draw pixmap with adjusted transform (no 4x scale)
```

### `rasterize_svg_to_premul_rgba8(tree, width, height) -> Vec<u8>`

Steps:

1. Allocate a `tiny_skia::Pixmap(width,height)`
2. Compute `sx,sy` to map SVG logical size → pixmap size
3. `resvg::render(tree, Transform::from_scale(sx,sy), pixmap)`
4. Return pixmap bytes

Note:

- resvg renders into premultiplied RGBA8 in tiny-skia pixmaps.

---

<!-- CHUNK 6 END -->


# CHUNK 7 — rendering + compile + execution modules (overview-by-file)

At this point, we have fully explained:

- error/core/anim/transitions/model/fx/eval
- the asset boundary (`AssetCache`) and SVG rasterization policy

The remaining `src/` files are mostly about:

- how we compile evaluated nodes into a backend-agnostic IR (`RenderPlan`)
- how each backend executes that IR (CPU/GPU)
- how we stream frames to `ffmpeg` for MP4 output
- how builders and the CLI wrap the core API

This chunk is more “systems overview” than line-by-line, because these files are large and are the
most likely refactor targets. The goal is still that **every struct/function is mentioned** and
placed into the dataflow.

---

## File: `src/blur_cpu.rs`

### Purpose

Provides a CPU blur implementation over premultiplied RGBA8 buffers.

Key API:

- `pub fn blur_rgba8_premul(src, width, height, radius, sigma) -> Vec<u8>`

Core algorithm:

- separable Gaussian blur:
  - build 1D kernel (Q16 fixed-point weights)
  - horizontal pass into temp buffer
  - vertical pass into output buffer

Key internal helpers:

- `gaussian_kernel_q16(radius, sigma) -> Vec<u32>`
  - builds weights, normalizes to sum≈65536, fixes rounding drift by adjusting center tap
- `horizontal_pass(...)`, `vertical_pass(...)`
  - clamp-to-edge sampling for out-of-bounds taps
- `q16_to_u8(acc)`
  - converts accumulated Q16 sum back to u8 with rounding

Tests:

- radius 0 is identity
- constant image stays constant
- single pixel spreads but conserves alpha approximately

---

## File: `src/composite_cpu.rs`

### Purpose

CPU compositing primitives on premultiplied RGBA8:

- “over” (src-over-dst)
- “crossfade”
- “wipe” (directional with optional soft edge)

Public surface APIs:

- `pub type PremulRgba8 = [u8; 4]`
- `pub fn over(dst_px, src_px, opacity) -> PremulRgba8`
- `pub fn crossfade(a_px, b_px, t) -> PremulRgba8`
- `pub fn over_in_place(dst_buf, src_buf, opacity) -> Result<()>`
- `pub fn crossfade_over_in_place(dst_buf, a_buf, b_buf, t) -> Result<()>`
- `pub fn wipe_over_in_place(dst_buf, a_buf, b_buf, params: WipeParams) -> Result<()>`
- `pub struct WipeParams { width, height, t, dir, soft_edge }`

Important compositing invariants:

- Inputs are premultiplied.
- Opacity/t values are clamped to [0,1].
- Buffer sizes are validated before operating.

Key internal math helpers:

- `mul_div255` implements `(x*y + 127)/255` (byte-scale multiply)
- `smoothstep` used for soft edge band blending in wipes

Tests cover:

- over edge cases (opacity 0, alpha 0, opaque replacement)
- crossfade endpoints
- wipe endpoints, midpoint behavior, and soft-edge behavior

---

## File: `src/compile.rs`

### Purpose

The compiler takes an `EvaluatedGraph` (one frame) and produces a backend-agnostic `RenderPlan`.

`RenderPlan` is the central IR of v0.1:

- explicit surfaces
- explicit passes
- draw ops reference `AssetId` (stable id) rather than file paths

Key IR types (all public):

- `RenderPlan { canvas, surfaces, passes, final_surface }`
- `Pass`:
  - `Scene(ScenePass)` (draw ops into a surface)
  - `Offscreen(OffscreenPass)` (pass effect; input->output)
  - `Composite(CompositePass)` (combine surfaces into target)
- `SurfaceId(u32)`, `SurfaceDesc { width, height, format }`, `PixelFormat::Rgba8Premul`
- `DrawOp`:
  - `FillPath { path, transform, color, opacity, blend, z }`
  - `Image { asset, transform, opacity, blend, z }`
  - `Svg { asset, transform, opacity, blend, z }`
  - `Text { asset, transform, opacity, blend, z }`
- `CompositeOp`:
  - `Over { src, opacity }`
  - `Crossfade { a, b, t }`
  - `Wipe { a, b, t, dir, soft_edge }`

Key entry point:

- `pub fn compile_frame(comp, eval, assets) -> RenderPlan`

High-level algorithm:

1. Declare surface 0 as the “final canvas surface”.
2. For each evaluated node:
   - parse + normalize effects (`fx.rs`)
   - fold inline effects into opacity + transform
   - skip if opacity <= 0
   - emit a `DrawOp`:
     - Path parses svg `d` into `BezPath` (`parse_svg_path`)
     - Image/Svg/Text produce `AssetId` via `assets.id_for(asset)`
   - allocate a new surface for that layer
   - emit a `ScenePass` drawing the op into the layer surface
   - for each pass effect (e.g. blur):
     - allocate an additional surface
     - emit an `OffscreenPass` chaining input->output
   - record the final surface for this node in a `Layer` list
3. Turn `Layer` list into `CompositeOp`s:
   - v0.1 tries to pair transitions:
     - Out of layer i with In of layer i+1
     - pairing only if progress is “close” (|t_in - t_out| <= 0.05)
     - supports Crossfade↔Crossfade and Wipe↔Wipe (dir+soft_edge must match)
   - if paired:
     - emit `CompositeOp::Crossfade` or `CompositeOp::Wipe`
     - advance by 2 layers
   - if not paired:
     - compute a fallback layer opacity from transitions:
       - multiply by `progress` for transition_in
       - multiply by `(1-progress)` for transition_out
     - emit `CompositeOp::Over` if opacity>0
4. Emit a final `CompositePass` onto surface 0.

Path parsing helper:

- `fn parse_svg_path(d) -> BezPath`
  - trims and rejects empty
  - uses `BezPath::from_svg(d)` and maps parse errors to validation errors

Tests in this file validate:

- path emission does not require AssetCache
- inline effects alter opacity/transform as expected
- blur creates an offscreen pass and composites the blurred output
- transition pairing rules and non-pairing behavior

---

## File: `src/render_passes.rs`

### Purpose

Defines the backend execution interface for `RenderPlan`.

Key types:

- `pub trait PassBackend`
  - `ensure_surface(id, desc)`
  - `exec_scene(scene_pass, assets)`
  - `exec_offscreen(offscreen_pass, assets)`
  - `exec_composite(composite_pass, assets)`
  - `readback_rgba8(surface, plan, assets) -> FrameRGBA`
- `pub fn execute_plan(backend, plan, assets) -> FrameRGBA`

`execute_plan` is backend-agnostic:

- It allocates/ensures all surfaces first.
- It runs passes in order.
- It performs a final readback of `plan.final_surface`.

The tests use a `MockBackend` to prove call ordering and output shape.

---

## File: `src/render_cpu.rs`

### Purpose

CPU backend implementation using `vello_cpu` for raster ops, plus our own:

- CPU blur (`blur_cpu.rs`)
- CPU compositing (`composite_cpu.rs`)
- SVG rasterization (`svg_raster.rs`)

Key types:

- `pub struct CpuBackend` (implements `PassBackend` and thus `RenderBackend`)
- internal `CpuSurface { width, height, pixmap }`

Caching inside `CpuBackend`:

- `image_cache: AssetId -> vello_cpu::Image`
- `svg_cache: SvgRasterKey -> vello_cpu::Image`
- `font_cache: AssetId -> vello_cpu::FontData`
- `surfaces: SurfaceId -> CpuSurface`

Important pass implementations:

- `ensure_surface`:
  - resets surfaces on SurfaceId(0) (new frame)
  - allocates pixmaps
  - applies clear color (premultiplied) on surface 0 if configured
- `exec_scene`:
  - clears target if requested
  - builds a `vello_cpu::RenderContext`
  - calls `draw_op(...)` for each `DrawOp`
  - flushes and renders into the pixmap
- `exec_offscreen`:
  - currently only blur
  - copies input bytes (handles in-place case)
  - runs `blur_rgba8_premul`, writes bytes back
- `exec_composite`:
  - runs Over/Crossfade/Wipe by delegating to `composite_cpu.rs`
- `readback_rgba8`:
  - returns pixmap bytes as `FrameRGBA` (premultiplied=true)

`draw_op` covers all `DrawOp` variants:

- FillPath:
  - converts `BezPath` into vello_cpu path elements
  - uses optional opacity layer
- Image:
  - loads prepared image, converts to a pixmap-based paint, draws a rect
- Text:
  - uses Parley layout items to emit glyph runs via `vello_cpu::RenderContext`
  - uses a cached `FontData` built from `PreparedText.font_bytes`
- Svg:
  - uses `svg_raster_params` + `rasterize_svg_to_premul_rgba8`
  - caches by `(AssetId,width,height)`
  - draws as an image rect with `transform_adjust`

Notable helpers:

- `premul_rgba8` used for clear color premultiplication
- conversion helpers (`affine_to_cpu`, `bezpath_to_cpu`, …)

---

## File: `src/render_vello.rs` (GPU backend, `--features gpu`)

### Purpose

GPU backend implementation using:

- `wgpu` for device/queue and compute/render pipelines
- `vello` for vector scene rendering

Important design point in v0.1:

- SVG is rasterized via `resvg` *even on GPU backend* for correctness.
  - Raster bytes are uploaded and drawn as an image in the GPU scene.

Key types (high-level):

- `pub struct VelloBackend`
  - implements `PassBackend` and thus `RenderBackend`
- internal surface/compute structs:
  - `GpuSurface`
  - `Compositor`
  - `BlurCompute`

Major responsibilities of `VelloBackend`:

- manage a `wgpu::Device` + `wgpu::Queue`
- allocate per-surface textures/buffers
- render `ScenePass` using vello on GPU
- perform compute shaders for:
  - composite ops (including wipe)
  - blur passes (GPU compute)
- readback final surface to CPU RGBA8

Important caches:

- image cache (uploaded textures)
- svg cache keyed by `SvgRasterKey` (uploaded raster textures)

Because this file is large, treat it as three conceptual blocks:

1. Device/surface setup and book-keeping
2. Scene rendering (vello) and image drawing path (including SVG)
3. Compute pipelines for compositing/blur and readback

---

## File: `src/render.rs`

### Purpose

Defines:

- the `FrameRGBA` output type
- the `RenderBackend` trait (extends `PassBackend` with a default `render_plan`)
- `BackendKind` enum
- `RenderSettings` (currently only clear color)
- `create_backend(kind, settings)` factory

This is the user-facing “backend selection” entry point.

---

## File: `src/encode_ffmpeg.rs`

### Purpose

MP4 encoding via the system `ffmpeg` binary by streaming raw RGBA frames to stdin.

Key types/functions:

- `EncodeConfig { width, height, fps, out_path, overwrite }`
  - `validate()` enforces:
    - non-zero dims
    - even width/height (yuv420p requirement)
    - non-zero fps
  - `with_out_path(...)` convenience
- `default_mp4_config(...)`
- `is_ffmpeg_on_path()`
- `ensure_parent_dir(path)`
- `pub struct FfmpegEncoder`
  - `new(cfg, bg_rgba)` spawns ffmpeg with configured args
  - `encode_frame(frame)` flattens alpha and writes bytes
  - `finish()` closes stdin and checks ffmpeg exit status

Alpha flattening:

- `flatten_to_opaque_rgba8(dst, src, src_is_premul, bg_rgba)`
  - supports both premultiplied and straight-alpha inputs
  - always outputs opaque RGBA (alpha=255) for encoder

Tests validate config checks and correct flattening math.

---

## File: `src/pipeline.rs`

### Purpose

Defines the high-level convenience APIs:

- `render_frame` (evaluate+compile+execute)
- `render_frames` (loop)
- `render_to_mp4` (loop + ffmpeg stream)

Key types:

- `RenderToMp4Opts { range, bg_rgba, overwrite }`

Important constraints:

- `render_to_mp4` currently requires integer fps (`fps.den == 1`)
- checks ffmpeg availability up-front
- uses `FfmpegEncoder` and calls `encode_frame` per rendered frame

---

## File: `src/dsl.rs`

### Purpose

Provides a builder-style DSL for programmatic composition construction.

Key builders:

- `CompositionBuilder`
  - sets fps/canvas/duration/seed
  - adds assets (rejects duplicate keys)
  - adds tracks
  - `build()` returns a validated `Composition`
- `TrackBuilder`
  - sets name/z_base
  - accumulates clips
  - validates non-empty name
- `ClipBuilder`
  - sets clip id/asset_key/range
  - default opacity=1, transform=identity, blend=Normal
  - attaches effects and transitions
  - validates id/asset key and validates Anim fields

The DSL intentionally does not do IO and does not prepare assets.

---

## File: `src/guide.rs`

### Purpose

`guide.rs` is a large rustdoc-only narrative module.

It is not “runtime logic”, but it is important because:

- it is the canonical long-form architecture explanation on docs.rs
- it documents invariants like “premultiplied RGBA8” and “no IO in renderer”

When refactoring, keep guide.rs in sync with actual behavior (especially backend/SVG handling).

---

## File: `src/bin/wavyte.rs` (CLI)

### Purpose

Defines the `wavyte` CLI binary using `clap`.

Subcommands:

- `frame`:
  - reads composition JSON
  - renders a single frame to PNG
- `render`:
  - renders full duration (or range) to MP4 (requires ffmpeg)

Backend selection:

- `--backend cpu` (default)
- `--backend gpu` (requires building with `--features gpu`)

Font/SVG diagnostics (uplift A):

- `--dump-fonts` prints text asset resolved family + sha256(font bytes)
- `--dump-svg-fonts` prints svg text node count + face count in svg fontdb

Key helper functions:

- `read_comp_json`
- `make_backend`
- `dump_font_diagnostics`
- `sha256_hex`
- `count_svg_text_nodes`

---

## File: `src/lib.rs` (crate root)

### Purpose

Defines:

- module declarations (`mod ...`)
- forbids unsafe (`#![forbid(unsafe_code)]`)
- public re-exports that define the crate’s public API surface

When navigating the crate:

- `lib.rs` tells you what is intended to be public vs internal.
- Most code is internal modules; the public API is the curated `pub use` list.

---

<!-- CHUNK 7 END -->
