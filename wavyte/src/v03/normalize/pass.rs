use crate::foundation::core::{Canvas, Fps};
use crate::v03::animation::anim::{Anim, AnimTagged};
use crate::v03::foundation::ids::{AssetIdx, NodeIdx, VarId};
use crate::v03::normalize::intern::{InternId, StringInterner};
use crate::v03::normalize::ir::{
    AnimIR, AssetIR, CollectionModeIR, CompositionIR, EffectInstanceIR, InterpModeIR, KeyframeIR,
    KeyframesIR, LayoutIR, MaskIR, MaskModeIR, MaskSourceIR, NodeIR, NodeKindIR, NodePropsIR,
    NormalizedComposition, RegistryBindings, ShapeIR, TransitionSpecIR, VarValueIR,
};
use crate::v03::scene::model::{
    AssetDef, CollectionModeDef, CompositionDef, EffectInstanceDef, MaskDef, MaskModeDef,
    MaskSourceDef, NodeDef, NodeKindDef, ShapeDef, TransitionSpecDef, VarDef, Vec2Def,
};
use crate::v03::schema::validate::{SchemaErrors, validate_composition};
use std::collections::HashMap;
use std::ops::Range;

#[derive(Debug)]
pub(crate) enum NormalizeError {
    Schema(SchemaErrors),
    TooManyNodes,
    TooManyAssets,
    TooManyVars,
}

impl std::fmt::Display for NormalizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Schema(e) => write!(f, "{e}"),
            Self::TooManyNodes => write!(f, "too many nodes"),
            Self::TooManyAssets => write!(f, "too many assets"),
            Self::TooManyVars => write!(f, "too many vars"),
        }
    }
}

impl std::error::Error for NormalizeError {}

pub(crate) fn normalize(def: &CompositionDef) -> Result<NormalizedComposition, NormalizeError> {
    validate_composition(def).map_err(NormalizeError::Schema)?;

    let mut interner = StringInterner::new();

    // Assets: deterministic order by BTreeMap key ordering.
    let mut asset_key_by_idx = Vec::with_capacity(def.assets.len());
    let mut asset_idx_by_key = HashMap::<InternId, AssetIdx>::with_capacity(def.assets.len());
    let mut assets = Vec::with_capacity(def.assets.len());
    for (i, (k, a)) in def.assets.iter().enumerate() {
        let idx = AssetIdx(u32::try_from(i).map_err(|_| NormalizeError::TooManyAssets)?);
        let key_i = interner.intern(k);
        asset_key_by_idx.push(key_i);
        asset_idx_by_key.insert(key_i, idx);
        assets.push(normalize_asset(a, &mut interner));
    }

    // Vars: deterministic order by BTreeMap key ordering.
    let mut var_key_by_id = Vec::with_capacity(def.variables.len());
    let mut var_id_by_key = HashMap::<InternId, VarId>::with_capacity(def.variables.len());
    let mut vars = Vec::with_capacity(def.variables.len());
    for (i, (k, v)) in def.variables.iter().enumerate() {
        let id = VarId(u16::try_from(i).map_err(|_| NormalizeError::TooManyVars)?);
        let key_i = interner.intern(k);
        var_key_by_id.push(key_i);
        var_id_by_key.insert(key_i, id);
        vars.push(normalize_var_value(v, &mut interner));
    }

    // Nodes: DFS preorder allocation is deterministic.
    let mut nodes = Vec::<NodeIR>::new();
    let mut node_id_by_idx = Vec::<InternId>::new();
    let mut node_idx_by_id = HashMap::<InternId, NodeIdx>::new();

    let root = normalize_node(
        &def.root,
        &mut interner,
        &asset_idx_by_key,
        &mut nodes,
        &mut node_id_by_idx,
        &mut node_idx_by_id,
    )?;

    let ir = CompositionIR {
        canvas: Canvas {
            width: def.canvas.width,
            height: def.canvas.height,
        },
        fps: Fps {
            num: def.fps.num,
            den: def.fps.den,
        },
        duration_frames: def.duration,
        seed: def.seed,
        vars,
        assets,
        nodes,
        root,
        layout: LayoutIR::default(),
        registries: RegistryBindings::default(),
    };

    Ok(NormalizedComposition {
        ir,
        interner,
        node_id_by_idx,
        node_idx_by_id,
        asset_key_by_idx,
        asset_idx_by_key,
        var_key_by_id,
        var_id_by_key,
    })
}

fn normalize_asset(a: &AssetDef, interner: &mut StringInterner) -> AssetIR {
    match a {
        AssetDef::Image { source } => AssetIR::Image {
            source: interner.intern(source),
        },
        AssetDef::Svg { source } => AssetIR::Svg {
            source: interner.intern(source),
        },
        AssetDef::Path { svg_path_d } => AssetIR::Path {
            svg_path_d: interner.intern(svg_path_d),
        },
        AssetDef::Text {
            text,
            font_source,
            size_px,
            max_width_px,
            color,
        } => AssetIR::Text {
            text: interner.intern(text),
            font_source: interner.intern(font_source),
            size_px: *size_px,
            max_width_px: *max_width_px,
            color: color.as_ref().map(|c| normalize_var_value(c, interner)),
        },
        AssetDef::Video {
            source,
            trim_start_sec,
            trim_end_sec,
            playback_rate,
        } => AssetIR::Video {
            source: interner.intern(source),
            trim_start_sec: *trim_start_sec,
            trim_end_sec: *trim_end_sec,
            playback_rate: *playback_rate,
        },
        AssetDef::Audio {
            source,
            trim_start_sec,
            trim_end_sec,
            playback_rate,
        } => AssetIR::Audio {
            source: interner.intern(source),
            trim_start_sec: *trim_start_sec,
            trim_end_sec: *trim_end_sec,
            playback_rate: *playback_rate,
        },
        AssetDef::Null => AssetIR::Null,
    }
}

fn normalize_var_value(v: &VarDef, interner: &mut StringInterner) -> VarValueIR {
    match v {
        VarDef::Bool(b) => VarValueIR::Bool(*b),
        VarDef::F64(x) => VarValueIR::F64(*x),
        VarDef::Vec2(v) => VarValueIR::Vec2 { x: v.x, y: v.y },
        VarDef::HexColor(s) => VarValueIR::HexColor(interner.intern(s)),
    }
}

fn normalize_node(
    node: &NodeDef,
    interner: &mut StringInterner,
    asset_idx_by_key: &HashMap<InternId, AssetIdx>,
    nodes: &mut Vec<NodeIR>,
    node_id_by_idx: &mut Vec<InternId>,
    node_idx_by_id: &mut HashMap<InternId, NodeIdx>,
) -> Result<NodeIdx, NormalizeError> {
    let idx_u32 = u32::try_from(nodes.len()).map_err(|_| NormalizeError::TooManyNodes)?;
    let idx = NodeIdx(idx_u32);

    let id = interner.intern(&node.id);
    node_id_by_idx.push(id);
    node_idx_by_id.insert(id, idx);

    let range = Range {
        start: node.range[0],
        end: node.range[1],
    };

    let effects = node
        .effects
        .iter()
        .map(|e| normalize_effect(e, interner))
        .collect();
    let mask = node.mask.as_ref().map(|m| normalize_mask(m, interner));
    let transition_in = node
        .transition_in
        .as_ref()
        .map(|t| normalize_transition(t, interner));
    let transition_out = node
        .transition_out
        .as_ref()
        .map(|t| normalize_transition(t, interner));

    // Push a placeholder so children can allocate stable indices.
    nodes.push(NodeIR {
        id,
        range,
        kind: NodeKindIR::CompRef {
            composition: serde_json::Value::Null,
        },
        props: NodePropsIR {
            opacity: AnimIR::Constant(1.0),
            translate: AnimIR::Constant((0.0, 0.0)),
            rotation_deg: AnimIR::Constant(0.0),
            scale: AnimIR::Constant((1.0, 1.0)),
            anchor: AnimIR::Constant((0.0, 0.0)),
            skew_deg: AnimIR::Constant((0.0, 0.0)),
            switch_active: None,
        },
        effects,
        mask,
        transition_in,
        transition_out,
    });

    let (kind, props) = normalize_kind_and_props(
        node,
        interner,
        asset_idx_by_key,
        nodes,
        node_id_by_idx,
        node_idx_by_id,
    )?;
    nodes[idx.0 as usize].kind = kind;
    nodes[idx.0 as usize].props = props;

    Ok(idx)
}

fn normalize_kind_and_props(
    node: &NodeDef,
    interner: &mut StringInterner,
    asset_idx_by_key: &HashMap<InternId, AssetIdx>,
    nodes: &mut Vec<NodeIR>,
    node_id_by_idx: &mut Vec<InternId>,
    node_idx_by_id: &mut HashMap<InternId, NodeIdx>,
) -> Result<(NodeKindIR, NodePropsIR), NormalizeError> {
    let opacity = normalize_anim_f64(&node.opacity, interner);
    let translate = normalize_anim_vec2(&node.transform.translate, interner);
    let rotation_deg = normalize_anim_f64(&node.transform.rotation_deg, interner);
    let scale = normalize_anim_vec2(&node.transform.scale, interner);
    let anchor = normalize_anim_vec2(&node.transform.anchor, interner);
    let skew_deg = normalize_anim_vec2(&node.transform.skew_deg, interner);

    let mut props = NodePropsIR {
        opacity,
        translate,
        rotation_deg,
        scale,
        anchor,
        skew_deg,
        switch_active: None,
    };

    let kind = match &node.kind {
        NodeKindDef::Leaf { asset } => {
            let asset_i = interner.intern(asset);
            let idx = asset_idx_by_key[&asset_i];
            NodeKindIR::Leaf { asset: idx }
        }
        NodeKindDef::Collection { mode, children } => {
            let mut child_idxs = Vec::with_capacity(children.len());
            for child in children {
                let child_idx = normalize_node(
                    child,
                    interner,
                    asset_idx_by_key,
                    nodes,
                    node_id_by_idx,
                    node_idx_by_id,
                )?;
                child_idxs.push(child_idx);
            }

            let mut sequence_prefix_starts = None;
            let mode_ir = match mode {
                CollectionModeDef::Group => CollectionModeIR::Group,
                CollectionModeDef::Sequence => {
                    let mut prefix = Vec::with_capacity(child_idxs.len() + 1);
                    prefix.push(0);
                    let mut acc = 0u64;
                    for child in children {
                        let dur = child.range[1];
                        acc = acc.saturating_add(dur);
                        prefix.push(acc);
                    }
                    sequence_prefix_starts = Some(prefix);
                    CollectionModeIR::Sequence
                }
                CollectionModeDef::Stack => CollectionModeIR::Stack,
                CollectionModeDef::Switch { active } => {
                    props.switch_active = Some(normalize_anim_u64(active, interner));
                    CollectionModeIR::Switch
                }
            };

            NodeKindIR::Collection {
                mode: mode_ir,
                children: child_idxs,
                sequence_prefix_starts,
            }
        }
        NodeKindDef::CompRef { composition } => NodeKindIR::CompRef {
            composition: composition.clone(),
        },
    };

    Ok((kind, props))
}

fn normalize_effect(e: &EffectInstanceDef, interner: &mut StringInterner) -> EffectInstanceIR {
    let kind = interner.intern(&e.kind);
    let params = e
        .params
        .iter()
        .map(|(k, v)| (interner.intern(k), v.clone()))
        .collect();
    EffectInstanceIR { kind, params }
}

fn normalize_transition(t: &TransitionSpecDef, interner: &mut StringInterner) -> TransitionSpecIR {
    let kind = interner.intern(&t.kind);
    let ease = t.ease.as_ref().map(|e| interner.intern(e));
    let params = t
        .params
        .iter()
        .map(|(k, v)| (interner.intern(k), v.clone()))
        .collect();
    TransitionSpecIR {
        kind,
        duration_frames: t.duration_frames,
        ease,
        params,
    }
}

fn normalize_mask(m: &MaskDef, interner: &mut StringInterner) -> MaskIR {
    MaskIR {
        source: match &m.source {
            MaskSourceDef::Node(id) => MaskSourceIR::Node(interner.intern(id)),
            MaskSourceDef::Asset(key) => MaskSourceIR::Asset(interner.intern(key)),
            MaskSourceDef::Shape(s) => MaskSourceIR::Shape(normalize_shape(s, interner)),
        },
        mode: match m.mode {
            MaskModeDef::Alpha => MaskModeIR::Alpha,
            MaskModeDef::Luma => MaskModeIR::Luma,
            MaskModeDef::Stencil { threshold } => MaskModeIR::Stencil { threshold },
        },
        inverted: m.inverted,
    }
}

fn normalize_shape(s: &ShapeDef, interner: &mut StringInterner) -> ShapeIR {
    match s {
        ShapeDef::Rect { width, height } => ShapeIR::Rect {
            width: *width,
            height: *height,
        },
        ShapeDef::RoundedRect {
            width,
            height,
            radius,
        } => ShapeIR::RoundedRect {
            width: *width,
            height: *height,
            radius: *radius,
        },
        ShapeDef::Ellipse { rx, ry } => ShapeIR::Ellipse { rx: *rx, ry: *ry },
        ShapeDef::Path { svg_path_d } => ShapeIR::Path {
            svg_path_d: interner.intern(svg_path_d),
        },
    }
}

fn normalize_anim_f64(a: &Anim<f64>, interner: &mut StringInterner) -> AnimIR<f64> {
    match a {
        Anim::Constant(x) => AnimIR::Constant(*x),
        Anim::Tagged(t) => match t {
            AnimTagged::Keyframes(k) => AnimIR::Keyframes(KeyframesIR {
                keys: k
                    .keys
                    .iter()
                    .map(|k| KeyframeIR {
                        frame: k.frame,
                        value: k.value,
                    })
                    .collect(),
                mode: match k.mode {
                    crate::v03::animation::anim::InterpMode::Linear => InterpModeIR::Linear,
                    crate::v03::animation::anim::InterpMode::Hold => InterpModeIR::Hold,
                },
            }),
            AnimTagged::Expr(s) => AnimIR::Expr(interner.intern(s)),
        },
    }
}

fn normalize_anim_u64(a: &Anim<u64>, interner: &mut StringInterner) -> AnimIR<u64> {
    match a {
        Anim::Constant(x) => AnimIR::Constant(*x),
        Anim::Tagged(t) => match t {
            AnimTagged::Keyframes(k) => AnimIR::Keyframes(KeyframesIR {
                keys: k
                    .keys
                    .iter()
                    .map(|k| KeyframeIR {
                        frame: k.frame,
                        value: k.value,
                    })
                    .collect(),
                mode: match k.mode {
                    crate::v03::animation::anim::InterpMode::Linear => InterpModeIR::Linear,
                    crate::v03::animation::anim::InterpMode::Hold => InterpModeIR::Hold,
                },
            }),
            AnimTagged::Expr(s) => AnimIR::Expr(interner.intern(s)),
        },
    }
}

fn normalize_anim_vec2(a: &Anim<Vec2Def>, interner: &mut StringInterner) -> AnimIR<(f64, f64)> {
    match a {
        Anim::Constant(v) => AnimIR::Constant((v.x, v.y)),
        Anim::Tagged(t) => match t {
            AnimTagged::Keyframes(k) => AnimIR::Keyframes(KeyframesIR {
                keys: k
                    .keys
                    .iter()
                    .map(|k| KeyframeIR {
                        frame: k.frame,
                        value: (k.value.x, k.value.y),
                    })
                    .collect(),
                mode: match k.mode {
                    crate::v03::animation::anim::InterpMode::Linear => InterpModeIR::Linear,
                    crate::v03::animation::anim::InterpMode::Hold => InterpModeIR::Hold,
                },
            }),
            AnimTagged::Expr(s) => AnimIR::Expr(interner.intern(s)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v03::animation::anim::Anim;
    use crate::v03::scene::model::{CanvasDef, FpsDef, NodeKindDef, TransformDef};
    use std::collections::BTreeMap;

    #[test]
    fn normalize_builds_deterministic_node_arena() {
        let mut assets = BTreeMap::new();
        assets.insert(
            "a".to_owned(),
            AssetDef::Image {
                source: "x.png".to_owned(),
            },
        );

        let def = CompositionDef {
            version: "0.3".to_owned(),
            canvas: CanvasDef {
                width: 1280,
                height: 720,
            },
            fps: FpsDef { num: 30, den: 1 },
            duration: 10,
            seed: 123,
            variables: BTreeMap::new(),
            assets,
            root: NodeDef {
                id: "root".to_owned(),
                kind: NodeKindDef::Collection {
                    mode: CollectionModeDef::Group,
                    children: vec![NodeDef {
                        id: "c".to_owned(),
                        kind: NodeKindDef::Leaf {
                            asset: "a".to_owned(),
                        },
                        range: [0, 10],
                        transform: TransformDef::default(),
                        opacity: Anim::Constant(1.0),
                        effects: vec![],
                        mask: None,
                        transition_in: None,
                        transition_out: None,
                    }],
                },
                range: [0, 10],
                transform: TransformDef::default(),
                opacity: Anim::Constant(1.0),
                effects: vec![],
                mask: None,
                transition_in: None,
                transition_out: None,
            },
        };

        let norm = normalize(&def).unwrap();
        assert_eq!(norm.ir.nodes.len(), 2);
        assert_eq!(norm.ir.root.0, 0);
        assert_eq!(norm.node_id_by_idx.len(), 2);
    }
}
