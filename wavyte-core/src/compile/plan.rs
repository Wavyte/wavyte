use std::collections::HashMap;

use crate::{
    assets::store::{AssetId, PreparedAsset, PreparedAssetStore},
    composition::model::{BlendMode, Composition},
    effects::fx::{PassFx, normalize_effects, parse_effect},
    effects::transitions::{TransitionKind, WipeDir, parse_transition_kind_params},
    eval::EvaluatedGraph,
    foundation::core::{Affine, BezPath, Canvas, Rgba8Premul},
    foundation::error::WavyteResult,
    foundation::math::Fnv1a64,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct JsonFingerprint {
    hi: u64,
    lo: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct EffectCacheKey {
    kind_hash: u64,
    params_fingerprint: JsonFingerprint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TransitionCacheKey {
    kind_hash: u64,
    params_fingerprint: JsonFingerprint,
}

#[derive(Clone, Debug)]
struct EffectCacheEntry {
    kind: String,
    params: serde_json::Value,
    parsed: crate::effects::fx::Effect,
}

#[derive(Clone, Debug)]
struct TransitionCacheEntry {
    kind: String,
    params: serde_json::Value,
    parsed: TransitionKind,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CompileCache {
    effect_cache: HashMap<EffectCacheKey, Vec<EffectCacheEntry>>,
    transition_cache: HashMap<TransitionCacheKey, Vec<TransitionCacheEntry>>,
}

#[derive(Clone, Debug)]
/// Backend-agnostic render plan for a single frame.
///
/// A plan consists of:
/// - surface declarations (`surfaces`)
/// - a sequence of passes (`passes`)
/// - a declared final surface (`final_surface`)
///
/// The plan is designed to be executable by multiple backends (CPU and GPU) with the same
/// semantics.
pub struct RenderPlan {
    pub canvas: Canvas,
    pub surfaces: Vec<SurfaceDesc>,
    pub passes: Vec<Pass>,
    pub final_surface: SurfaceId,
}

#[derive(Clone, Debug)]
/// A single pass in a [`RenderPlan`].
pub enum Pass {
    Scene(ScenePass),
    Offscreen(OffscreenPass),
    Composite(CompositePass),
}

#[derive(Clone, Debug)]
/// Draw operations into a surface.
pub struct ScenePass {
    pub target: SurfaceId,
    pub ops: Vec<DrawOp>,
    pub clear_to_transparent: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
/// Identifier for a render surface declared in [`RenderPlan::surfaces`].
pub struct SurfaceId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Supported pixel formats for render surfaces.
pub enum PixelFormat {
    Rgba8Premul,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Surface declaration: dimensions + pixel format.
pub struct SurfaceDesc {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
}

#[derive(Clone, Debug)]
/// Run a post-processing effect producing a new surface from an input surface.
pub struct OffscreenPass {
    pub input: SurfaceId,
    pub output: SurfaceId,
    pub fx: PassFx,
}

#[derive(Clone, Debug)]
/// Composite multiple surfaces into a target surface.
pub struct CompositePass {
    pub target: SurfaceId,
    pub ops: Vec<CompositeOp>,
}

#[derive(Clone, Debug)]
/// A compositing operation between surfaces.
pub enum CompositeOp {
    Over {
        src: SurfaceId,
        opacity: f32,
    },
    Crossfade {
        a: SurfaceId,
        b: SurfaceId,
        t: f32,
    },
    Wipe {
        a: SurfaceId,
        b: SurfaceId,
        t: f32,
        dir: WipeDir,
        soft_edge: f32,
    },
}

#[derive(Clone, Debug)]
/// Draw operation emitted by the compiler.
pub enum DrawOp {
    FillPath {
        path: BezPath,
        transform: Affine,
        color: Rgba8Premul,
        opacity: f32,
        blend: BlendMode,
        z: i32,
    },
    Image {
        asset: AssetId,
        transform: Affine,
        opacity: f32,
        blend: BlendMode,
        z: i32,
    },
    Svg {
        asset: AssetId,
        transform: Affine,
        opacity: f32,
        blend: BlendMode,
        z: i32,
    },
    Text {
        asset: AssetId,
        transform: Affine,
        opacity: f32,
        blend: BlendMode,
        z: i32,
    },
    Video {
        asset: AssetId,
        source_time_s: f64,
        transform: Affine,
        opacity: f32,
        blend: BlendMode,
        z: i32,
    },
}

pub fn compile_frame(
    comp: &Composition,
    eval: &EvaluatedGraph,
    assets: &PreparedAssetStore,
) -> WavyteResult<RenderPlan> {
    let mut cache = CompileCache::default();
    compile_frame_with_cache(comp, eval, assets, &mut cache)
}

pub(crate) fn compile_frame_with_cache(
    comp: &Composition,
    eval: &EvaluatedGraph,
    assets: &PreparedAssetStore,
    cache: &mut CompileCache,
) -> WavyteResult<RenderPlan> {
    #[derive(Clone, Debug)]
    struct Layer {
        surface: SurfaceId,
        transition_in: Option<crate::eval::ResolvedTransition>,
        transition_out: Option<crate::eval::ResolvedTransition>,
    }

    let mut surfaces = Vec::<SurfaceDesc>::new();
    surfaces.push(SurfaceDesc {
        width: comp.canvas.width,
        height: comp.canvas.height,
        format: PixelFormat::Rgba8Premul,
    });

    let mut scene_passes = Vec::<Pass>::with_capacity(eval.nodes.len());
    let mut layers = Vec::<Layer>::with_capacity(eval.nodes.len());

    for (idx, node) in eval.nodes.iter().enumerate() {
        let mut parsed = Vec::with_capacity(node.effects.len());
        for e in &node.effects {
            parsed.push(parse_effect_cached(cache, e)?);
        }
        let fx = normalize_effects(&parsed);

        // Transitions are handled during composition. Keep DrawOp opacity for "intrinsic" opacity
        // only (clip opacity + inline opacity effect).
        let opacity = ((node.opacity as f32) * fx.inline.opacity_mul).clamp(0.0, 1.0);

        if opacity <= 0.0 {
            continue;
        }

        let transform = node.transform * fx.inline.transform_post;

        let asset_id = assets.id_for_key(&node.asset)?;
        let op = match assets.get(asset_id)? {
            PreparedAsset::Path(a) => DrawOp::FillPath {
                path: a.path.clone(),
                transform,
                color: Rgba8Premul::from_straight_rgba(255, 255, 255, 255),
                opacity,
                blend: node.blend,
                z: node.z,
            },
            PreparedAsset::Image(_) => DrawOp::Image {
                asset: asset_id,
                transform,
                opacity,
                blend: node.blend,
                z: node.z,
            },
            PreparedAsset::Svg(_) => DrawOp::Svg {
                asset: asset_id,
                transform,
                opacity,
                blend: node.blend,
                z: node.z,
            },
            PreparedAsset::Text(_) => DrawOp::Text {
                asset: asset_id,
                transform,
                opacity,
                blend: node.blend,
                z: node.z,
            },
            PreparedAsset::Video(_) => DrawOp::Video {
                asset: asset_id,
                source_time_s: node.source_time_s.unwrap_or(0.0),
                transform,
                opacity,
                blend: node.blend,
                z: node.z,
            },
            PreparedAsset::Audio(_) => continue,
        };

        let surf_id = SurfaceId((surfaces.len()) as u32);
        surfaces.push(SurfaceDesc {
            width: comp.canvas.width,
            height: comp.canvas.height,
            format: PixelFormat::Rgba8Premul,
        });

        scene_passes.push(Pass::Scene(ScenePass {
            target: surf_id,
            ops: vec![op],
            clear_to_transparent: true,
        }));

        let mut post_fx = surf_id;
        for fx in &fx.passes {
            let out_id = SurfaceId((surfaces.len()) as u32);
            surfaces.push(SurfaceDesc {
                width: comp.canvas.width,
                height: comp.canvas.height,
                format: PixelFormat::Rgba8Premul,
            });
            scene_passes.push(Pass::Offscreen(OffscreenPass {
                input: post_fx,
                output: out_id,
                fx: fx.clone(),
            }));
            post_fx = out_id;
        }

        let _ = idx;
        layers.push(Layer {
            surface: post_fx,
            transition_in: node.transition_in.clone(),
            transition_out: node.transition_out.clone(),
        });
    }

    let mut composite_ops = Vec::<CompositeOp>::with_capacity(layers.len());
    let mut i = 0usize;
    while i < layers.len() {
        let layer = &layers[i];

        let mut paired = false;
        if i + 1 < layers.len() {
            let next = &layers[i + 1];

            if let (Some(out_tr), Some(in_tr)) =
                (layer.transition_out.as_ref(), next.transition_in.as_ref())
            {
                let out_kind = parse_transition_cached(cache, out_tr).ok();
                let in_kind = parse_transition_cached(cache, in_tr).ok();

                if let (Some(out_kind), Some(in_kind)) = (out_kind, in_kind) {
                    let t_in = (in_tr.progress as f32).clamp(0.0, 1.0);
                    let t_out = (out_tr.progress as f32).clamp(0.0, 1.0);

                    // Explicit v0.2 pairing rule: the Out and In edges must agree on progress
                    // (same duration/ease and overlapping window).
                    let progress_close = (t_in - t_out).abs() <= 0.05;

                    if progress_close {
                        match (out_kind, in_kind) {
                            (TransitionKind::Crossfade, TransitionKind::Crossfade) => {
                                composite_ops.push(CompositeOp::Crossfade {
                                    a: layer.surface,
                                    b: next.surface,
                                    t: t_in,
                                });
                                paired = true;
                            }
                            (
                                TransitionKind::Wipe {
                                    dir: dir_a,
                                    soft_edge: soft_a,
                                },
                                TransitionKind::Wipe {
                                    dir: dir_b,
                                    soft_edge: soft_b,
                                },
                            ) => {
                                if dir_a == dir_b && (soft_a - soft_b).abs() <= 1e-6 {
                                    composite_ops.push(CompositeOp::Wipe {
                                        a: layer.surface,
                                        b: next.surface,
                                        t: t_in,
                                        dir: dir_a,
                                        soft_edge: soft_a,
                                    });
                                    paired = true;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        if paired {
            i += 2;
            continue;
        }

        let mut layer_opacity = 1.0f32;
        if let Some(tr) = &layer.transition_in {
            layer_opacity *= tr.progress as f32;
        }
        if let Some(tr) = &layer.transition_out {
            layer_opacity *= (1.0 - tr.progress) as f32;
        }
        layer_opacity = layer_opacity.clamp(0.0, 1.0);

        if layer_opacity > 0.0 {
            composite_ops.push(CompositeOp::Over {
                src: layer.surface,
                opacity: layer_opacity,
            });
        }

        i += 1;
    }

    Ok(RenderPlan {
        canvas: comp.canvas,
        surfaces,
        passes: {
            let mut out = scene_passes;
            out.push(Pass::Composite(CompositePass {
                target: SurfaceId(0),
                ops: composite_ops,
            }));
            out
        },
        final_surface: SurfaceId(0),
    })
}

fn parse_effect_cached(
    cache: &mut CompileCache,
    effect: &crate::eval::ResolvedEffect,
) -> WavyteResult<crate::effects::fx::Effect> {
    let key = EffectCacheKey {
        kind_hash: hash_str64(&effect.kind),
        params_fingerprint: fingerprint_json_value(&effect.params),
    };

    if let Some(bucket) = cache.effect_cache.get(&key)
        && let Some(found) = bucket
            .iter()
            .find(|entry| entry.kind == effect.kind && entry.params == effect.params)
    {
        return Ok(found.parsed.clone());
    }

    let parsed = parse_effect(&crate::composition::model::EffectInstance {
        kind: effect.kind.clone(),
        params: effect.params.clone(),
    })?;
    cache
        .effect_cache
        .entry(key)
        .or_default()
        .push(EffectCacheEntry {
            kind: effect.kind.clone(),
            params: effect.params.clone(),
            parsed: parsed.clone(),
        });
    Ok(parsed)
}

fn parse_transition_cached(
    cache: &mut CompileCache,
    transition: &crate::eval::ResolvedTransition,
) -> WavyteResult<TransitionKind> {
    let key = TransitionCacheKey {
        kind_hash: hash_str64(&transition.kind),
        params_fingerprint: fingerprint_json_value(&transition.params),
    };

    if let Some(bucket) = cache.transition_cache.get(&key)
        && let Some(found) = bucket
            .iter()
            .find(|entry| entry.kind == transition.kind && entry.params == transition.params)
    {
        return Ok(found.parsed.clone());
    }

    let parsed = parse_transition_kind_params(&transition.kind, &transition.params)?;
    cache
        .transition_cache
        .entry(key)
        .or_default()
        .push(TransitionCacheEntry {
            kind: transition.kind.clone(),
            params: transition.params.clone(),
            parsed: parsed.clone(),
        });
    Ok(parsed)
}

fn hash_str64(value: &str) -> u64 {
    let mut h = Fnv1a64::new_default();
    h.write_bytes(value.as_bytes());
    h.finish()
}

fn fingerprint_json_value(v: &serde_json::Value) -> JsonFingerprint {
    let mut a = Fnv1a64::new(0xcbf29ce484222325);
    let mut b = Fnv1a64::new(0x9ae16a3b2f90404f);
    hash_json_value_pair(&mut a, &mut b, v);
    JsonFingerprint {
        hi: a.finish(),
        lo: b.finish(),
    }
}

fn hash_json_value_pair(a: &mut Fnv1a64, b: &mut Fnv1a64, v: &serde_json::Value) {
    match v {
        serde_json::Value::Null => write_u8_pair(a, b, 0),
        serde_json::Value::Bool(x) => {
            write_u8_pair(a, b, 1);
            write_u8_pair(a, b, u8::from(*x));
        }
        serde_json::Value::Number(n) => {
            write_u8_pair(a, b, 2);
            if let Some(i) = n.as_i64() {
                write_u8_pair(a, b, 0);
                write_i64_pair(a, b, i);
            } else if let Some(u) = n.as_u64() {
                write_u8_pair(a, b, 1);
                write_u64_pair(a, b, u);
            } else if let Some(f) = n.as_f64() {
                write_u8_pair(a, b, 2);
                write_u64_pair(a, b, f.to_bits());
            } else {
                write_u8_pair(a, b, 3);
                write_str_pair(a, b, &n.to_string());
            }
        }
        serde_json::Value::String(s) => {
            write_u8_pair(a, b, 3);
            write_str_pair(a, b, s);
        }
        serde_json::Value::Array(items) => {
            write_u8_pair(a, b, 4);
            write_u64_pair(a, b, items.len() as u64);
            for item in items {
                hash_json_value_pair(a, b, item);
            }
        }
        serde_json::Value::Object(map) => {
            write_u8_pair(a, b, 5);
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            write_u64_pair(a, b, keys.len() as u64);
            for k in keys {
                write_str_pair(a, b, k);
                hash_json_value_pair(a, b, &map[k]);
            }
        }
    }
}

fn write_u8_pair(a: &mut Fnv1a64, b: &mut Fnv1a64, v: u8) {
    a.write_u8(v);
    b.write_u8(v);
}

fn write_u64_pair(a: &mut Fnv1a64, b: &mut Fnv1a64, v: u64) {
    a.write_u64(v);
    b.write_u64(v);
}

fn write_i64_pair(a: &mut Fnv1a64, b: &mut Fnv1a64, v: i64) {
    write_u64_pair(a, b, v as u64);
}

fn write_str_pair(a: &mut Fnv1a64, b: &mut Fnv1a64, s: &str) {
    write_u64_pair(a, b, s.len() as u64);
    a.write_bytes(s.as_bytes());
    b.write_bytes(s.as_bytes());
}

#[cfg(test)]
#[path = "../../tests/unit/compile/plan.rs"]
mod tests;
