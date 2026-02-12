use crate::animation::anim::{
    Anim, AnimDef, AnimTaggedDef, InterpMode, Keyframe, Keyframes, Procedural, ProceduralKind,
};
use crate::assets::color::ColorDef;
use crate::effects::binding::{
    CoreEffectKindIR, CoreParamKeyIR, EffectBindingIR, ResolvedParamIR, ResolvedParamValueIR,
};
use crate::foundation::core::{Canvas, Fps};
use crate::foundation::ids::{AssetIdx, NodeIdx, VarId};
use crate::foundation::ids::{EffectKindId, ParamId};
use crate::normalize::intern::{InternId, StringInterner};
use crate::normalize::ir::{
    AnimDimensionIR, AssetIR, BlendModeIR, CollectionModeIR, CompositionIR, EdgesAnimIR,
    ExprSourceIR, IrisShapeIR, LayoutAlignContentIR, LayoutAlignItemsIR, LayoutDirectionIR,
    LayoutDisplayIR, LayoutIR, LayoutJustifyContentIR, LayoutPositionIR, LayoutPropsIR,
    LayoutWrapIR, MaskIR, MaskModeIR, MaskSourceIR, NodeIR, NodeKindIR, NodePropsIR,
    NormalizedComposition, RegistryBindings, ShapeIR, SizeAnimIR, SlideDirIR, TransitionKindIR,
    TransitionSpecIR, ValueTypeIR, VarValueIR, WipeDirIR,
};
use crate::normalize::property::{PropertyIndex, PropertyKey};
use crate::scene::model::{
    AnimDimensionDef, AssetDef, BlendModeDef, CollectionModeDef, CompositionDef, EdgesAnimDef,
    EffectInstanceDef, LayoutAlignContentDef, LayoutAlignItemsDef, LayoutDirectionDef,
    LayoutDisplayDef, LayoutJustifyContentDef, LayoutPositionDef, LayoutPropsDef, LayoutWrapDef,
    MaskDef, MaskModeDef, MaskSourceDef, NodeDef, NodeKindDef, ShapeDef, SizeDef, TransformDef,
    TransitionSpecDef, VarDef,
};
use crate::schema::validate::{SchemaErrors, validate_composition};
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
    let mut expr_sources = Vec::<ExprSourceIR>::new();
    let mut registries_builder = RegistryBindingsBuilder::default();

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
        vars.push(normalize_var_value(v));
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
        &mut expr_sources,
        &mut registries_builder,
    )?;

    let registries = registries_builder.build(&interner);

    let mut asset_idx_by_intern: Vec<Option<AssetIdx>> = vec![None; interner.len()];
    for (i, &k) in asset_key_by_idx.iter().enumerate() {
        if let Some(slot) = asset_idx_by_intern.get_mut(k.0 as usize) {
            *slot = Some(AssetIdx(u32::try_from(i).unwrap()));
        }
    }

    let mut node_idx_by_intern: Vec<Option<NodeIdx>> = vec![None; interner.len()];
    for (i, &id) in node_id_by_idx.iter().enumerate() {
        if let Some(slot) = node_idx_by_intern.get_mut(id.0 as usize) {
            *slot = Some(NodeIdx(u32::try_from(i).unwrap()));
        }
    }

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
        asset_idx_by_intern,
        nodes,
        root,
        node_idx_by_intern,
        layout: LayoutIR::default(),
        registries,
    };

    Ok(NormalizedComposition {
        ir,
        interner,
        expr_sources,
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
            color: color.as_ref().map(normalize_color_value),
        },
        AssetDef::Video {
            source,
            trim_start_sec,
            trim_end_sec,
            playback_rate,
            volume,
            mute,
            fade_in_sec,
            fade_out_sec,
        } => AssetIR::Video {
            source: interner.intern(source),
            trim_start_sec: *trim_start_sec,
            trim_end_sec: *trim_end_sec,
            playback_rate: *playback_rate,
            volume: *volume,
            mute: *mute,
            fade_in_sec: *fade_in_sec,
            fade_out_sec: *fade_out_sec,
        },
        AssetDef::Audio {
            source,
            trim_start_sec,
            trim_end_sec,
            playback_rate,
            volume,
            mute,
            fade_in_sec,
            fade_out_sec,
        } => AssetIR::Audio {
            source: interner.intern(source),
            trim_start_sec: *trim_start_sec,
            trim_end_sec: *trim_end_sec,
            playback_rate: *playback_rate,
            volume: *volume,
            mute: *mute,
            fade_in_sec: *fade_in_sec,
            fade_out_sec: *fade_out_sec,
        },
        AssetDef::SolidRect { color } => AssetIR::SolidRect {
            color: color.as_ref().map(normalize_color_value),
        },
        AssetDef::Gradient { start, end } => AssetIR::Gradient {
            start: normalize_color_value(start),
            end: normalize_color_value(end),
        },
        AssetDef::Noise { seed } => AssetIR::Noise { seed: *seed },
        AssetDef::Null => AssetIR::Null,
    }
}

fn normalize_var_value(v: &VarDef) -> VarValueIR {
    match v {
        VarDef::Bool(b) => VarValueIR::Bool(*b),
        VarDef::F64(x) => VarValueIR::F64(*x),
        VarDef::Vec2(v) => VarValueIR::Vec2 { x: v.x, y: v.y },
        VarDef::Color(c) => VarValueIR::Color(c.to_rgba8_premul()),
    }
}

fn normalize_color_value(c: &ColorDef) -> VarValueIR {
    VarValueIR::Color(c.to_rgba8_premul())
}

#[derive(Debug, Default)]
struct RegistryBindingsBuilder {
    effect_kind_id_by_intern: HashMap<InternId, EffectKindId>,
    effect_intern_by_id: Vec<InternId>,

    param_id_by_intern: HashMap<InternId, ParamId>,
    param_intern_by_id: Vec<InternId>,
}

impl RegistryBindingsBuilder {
    fn effect_kind_id(&mut self, kind: InternId) -> EffectKindId {
        if let Some(&id) = self.effect_kind_id_by_intern.get(&kind) {
            return id;
        }
        let id = EffectKindId(u16::try_from(self.effect_intern_by_id.len()).unwrap_or(u16::MAX));
        self.effect_intern_by_id.push(kind);
        self.effect_kind_id_by_intern.insert(kind, id);
        id
    }

    fn param_id(&mut self, key: InternId) -> ParamId {
        if let Some(&id) = self.param_id_by_intern.get(&key) {
            return id;
        }
        let id = ParamId(u16::try_from(self.param_intern_by_id.len()).unwrap_or(u16::MAX));
        self.param_intern_by_id.push(key);
        self.param_id_by_intern.insert(key, id);
        id
    }

    fn build(&self, interner: &StringInterner) -> RegistryBindings {
        let mut effect_kind_by_intern: Vec<Option<EffectKindId>> = vec![None; interner.len()];
        for (&k, &id) in &self.effect_kind_id_by_intern {
            if let Some(slot) = effect_kind_by_intern.get_mut(k.0 as usize) {
                *slot = Some(id);
            }
        }

        let mut param_id_by_intern: Vec<Option<ParamId>> = vec![None; interner.len()];
        for (&k, &id) in &self.param_id_by_intern {
            if let Some(slot) = param_id_by_intern.get_mut(k.0 as usize) {
                *slot = Some(id);
            }
        }

        RegistryBindings {
            effect_intern_by_id: self.effect_intern_by_id.clone(),
            effect_kind_by_intern,
            param_intern_by_id: self.param_intern_by_id.clone(),
            param_id_by_intern,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn normalize_node(
    node: &NodeDef,
    interner: &mut StringInterner,
    asset_idx_by_key: &HashMap<InternId, AssetIdx>,
    nodes: &mut Vec<NodeIR>,
    node_id_by_idx: &mut Vec<InternId>,
    node_idx_by_id: &mut HashMap<InternId, NodeIdx>,
    expr_sources: &mut Vec<ExprSourceIR>,
    registries: &mut RegistryBindingsBuilder,
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
        .map(|e| normalize_effect(e, interner, registries))
        .collect();
    let mask = node.mask.as_ref().map(|m| normalize_mask(m, interner));
    let transition_in = node.transition_in.as_ref().map(normalize_transition);
    let transition_out = node.transition_out.as_ref().map(normalize_transition);

    // Push a placeholder so children can allocate stable indices.
    nodes.push(NodeIR {
        id,
        range,
        kind: NodeKindIR::CompRef {
            composition: serde_json::Value::Null,
        },
        blend: normalize_blend(node.blend),
        props: NodePropsIR {
            opacity: Anim::Constant(1.0),
            translate_x: Anim::Constant(0.0),
            translate_y: Anim::Constant(0.0),
            rotation_rad: Anim::Constant(0.0),
            scale_x: Anim::Constant(1.0),
            scale_y: Anim::Constant(1.0),
            anchor_x: Anim::Constant(0.0),
            anchor_y: Anim::Constant(0.0),
            skew_x_deg: Anim::Constant(0.0),
            skew_y_deg: Anim::Constant(0.0),
            switch_active: None,
        },
        layout: None,
        effects,
        mask,
        transition_in,
        transition_out,
    });

    let (kind, props) = normalize_kind_and_props(
        idx,
        node,
        interner,
        asset_idx_by_key,
        nodes,
        node_id_by_idx,
        node_idx_by_id,
        expr_sources,
        registries,
    )?;
    nodes[idx.0 as usize].kind = kind;
    nodes[idx.0 as usize].props = props;
    nodes[idx.0 as usize].layout =
        normalize_layout(idx, node.layout.as_ref(), interner, expr_sources)?;

    Ok(idx)
}

#[allow(clippy::too_many_arguments)]
fn normalize_kind_and_props(
    self_idx: NodeIdx,
    node: &NodeDef,
    interner: &mut StringInterner,
    asset_idx_by_key: &HashMap<InternId, AssetIdx>,
    nodes: &mut Vec<NodeIR>,
    node_id_by_idx: &mut Vec<InternId>,
    node_idx_by_id: &mut HashMap<InternId, NodeIdx>,
    expr_sources: &mut Vec<ExprSourceIR>,
    registries: &mut RegistryBindingsBuilder,
) -> Result<(NodeKindIR, NodePropsIR), NormalizeError> {
    let mut props = normalize_props(
        self_idx,
        node.transform.clone(),
        &node.opacity,
        interner,
        expr_sources,
    )?;

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
                    expr_sources,
                    registries,
                )?;
                child_idxs.push(child_idx);
            }

            let mut sequence_prefix_starts = None;
            let mode_ir = match mode {
                CollectionModeDef::Group => CollectionModeIR::Group,
                CollectionModeDef::Sequence => {
                    fn overlap_between(a: &NodeDef, b: &NodeDef) -> u64 {
                        let dur_a = a.range[1].saturating_sub(a.range[0]);
                        let dur_b = b.range[1].saturating_sub(b.range[0]);

                        let overlap = match (a.transition_out.as_ref(), b.transition_in.as_ref()) {
                            (Some(ta), Some(tb))
                                if ta.kind == tb.kind
                                    && ta.duration_frames == tb.duration_frames
                                    && ta.duration_frames > 0 =>
                            {
                                ta.duration_frames as u64
                            }
                            (Some(ta), None) if ta.duration_frames > 0 => ta.duration_frames as u64,
                            (None, Some(tb)) if tb.duration_frames > 0 => tb.duration_frames as u64,
                            _ => 0,
                        };

                        overlap.min(dur_a).min(dur_b)
                    }

                    let mut prefix = Vec::with_capacity(child_idxs.len() + 1);
                    let mut acc = 0u64;
                    for (i, child) in children.iter().enumerate() {
                        prefix.push(acc);
                        let dur = child.range[1].saturating_sub(child.range[0]);
                        acc = acc.saturating_add(dur);
                        if let Some(next) = children.get(i + 1) {
                            let ov = overlap_between(child, next);
                            acc = acc.saturating_sub(ov);
                        }
                    }
                    prefix.push(acc);
                    sequence_prefix_starts = Some(prefix);
                    CollectionModeIR::Sequence
                }
                CollectionModeDef::Stack => CollectionModeIR::Stack,
                CollectionModeDef::Switch { active } => {
                    props.switch_active = Some(normalize_lane_u64(
                        self_idx,
                        PropertyKey::SwitchActiveIndex,
                        active,
                        interner,
                        expr_sources,
                    )?);
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

fn normalize_effect(
    e: &EffectInstanceDef,
    interner: &mut StringInterner,
    registries: &mut RegistryBindingsBuilder,
) -> EffectBindingIR {
    fn parse_effect_kind(kind: &str) -> CoreEffectKindIR {
        match kind.trim().to_ascii_lowercase().as_str() {
            "blur" => CoreEffectKindIR::Blur,
            "color_matrix" | "colormatrix" => CoreEffectKindIR::ColorMatrix,
            "drop_shadow" | "dropshadow" => CoreEffectKindIR::DropShadow,
            _ => CoreEffectKindIR::Unknown,
        }
    }

    fn parse_param_key(key: &str) -> CoreParamKeyIR {
        match key.trim().to_ascii_lowercase().as_str() {
            "value" => CoreParamKeyIR::Value,
            "radius_px" | "radius" => CoreParamKeyIR::RadiusPx,
            "sigma" => CoreParamKeyIR::Sigma,
            "matrix" => CoreParamKeyIR::Matrix,
            "offset" => CoreParamKeyIR::Offset,
            "color" => CoreParamKeyIR::Color,
            "blur_radius_px" => CoreParamKeyIR::BlurRadiusPx,
            _ => CoreParamKeyIR::Unknown,
        }
    }

    fn resolve_param_value(v: &serde_json::Value) -> ResolvedParamValueIR {
        if let Ok(c) = serde_json::from_value::<ColorDef>(v.clone()) {
            return ResolvedParamValueIR::Color(c.to_rgba8_premul());
        }

        match v {
            serde_json::Value::Number(n) => ResolvedParamValueIR::F64(n.as_f64().unwrap_or(0.0)),
            serde_json::Value::Bool(b) => ResolvedParamValueIR::Bool(*b),
            serde_json::Value::String(s) => ResolvedParamValueIR::String(s.clone()),
            serde_json::Value::Array(arr) => {
                if arr.len() == 2 {
                    let x = arr[0].as_f64();
                    let y = arr[1].as_f64();
                    if let (Some(x), Some(y)) = (x, y) {
                        return ResolvedParamValueIR::Vec2(crate::foundation::core::Vec2::new(
                            x, y,
                        ));
                    }
                }
                if arr.len() == 20 {
                    let mut m = [0.0f32; 20];
                    let mut ok = true;
                    for (i, v) in arr.iter().enumerate() {
                        let Some(f) = v.as_f64() else {
                            ok = false;
                            break;
                        };
                        m[i] = f as f32;
                    }
                    if ok {
                        return ResolvedParamValueIR::Matrix20(m);
                    }
                }
                ResolvedParamValueIR::Json(v.clone())
            }
            _ => ResolvedParamValueIR::Json(v.clone()),
        }
    }

    let kind_intern = interner.intern(&e.kind);
    let kind = registries.effect_kind_id(kind_intern);
    let core_kind = parse_effect_kind(&e.kind);

    let mut params = smallvec::SmallVec::<[ResolvedParamIR; 8]>::new();
    for (k, v) in &e.params {
        let key_intern = interner.intern(k);
        let id = registries.param_id(key_intern);
        params.push(ResolvedParamIR {
            id,
            key: parse_param_key(k),
            value: resolve_param_value(v),
        });
    }

    EffectBindingIR {
        kind,
        core_kind,
        params,
    }
}

fn normalize_blend(v: BlendModeDef) -> BlendModeIR {
    match v {
        BlendModeDef::Normal => BlendModeIR::Normal,
        BlendModeDef::Multiply => BlendModeIR::Multiply,
        BlendModeDef::Screen => BlendModeIR::Screen,
        BlendModeDef::Overlay => BlendModeIR::Overlay,
        BlendModeDef::Darken => BlendModeIR::Darken,
        BlendModeDef::Lighten => BlendModeIR::Lighten,
        BlendModeDef::ColorDodge => BlendModeIR::ColorDodge,
        BlendModeDef::ColorBurn => BlendModeIR::ColorBurn,
        BlendModeDef::SoftLight => BlendModeIR::SoftLight,
        BlendModeDef::HardLight => BlendModeIR::HardLight,
        BlendModeDef::Difference => BlendModeIR::Difference,
        BlendModeDef::Exclusion => BlendModeIR::Exclusion,
    }
}

fn normalize_transition(t: &TransitionSpecDef) -> TransitionSpecIR {
    let ease = match t.ease.as_deref() {
        None => InterpMode::Linear,
        Some("hold") => InterpMode::Hold,
        Some("linear") => InterpMode::Linear,
        Some("ease_in") => InterpMode::EaseIn,
        Some("ease_out") => InterpMode::EaseOut,
        Some("ease_in_out") => InterpMode::EaseInOut,
        Some("elastic_out") => InterpMode::ElasticOut,
        Some("bounce_out") => InterpMode::BounceOut,
        Some(_other) => InterpMode::Linear,
    };
    let kind_str = t.kind.trim().to_ascii_lowercase();
    let kind = match kind_str.as_str() {
        "crossfade" => TransitionKindIR::Crossfade,
        "wipe" => TransitionKindIR::Wipe {
            dir: parse_wipe_dir(t.params.get("dir").and_then(|v| v.as_str())),
            soft_edge: parse_soft_edge(t.params.get("soft_edge").and_then(|v| v.as_f64())),
        },
        "slide" => TransitionKindIR::Slide {
            dir: parse_slide_dir(t.params.get("dir").and_then(|v| v.as_str())),
            push: t
                .params
                .get("push")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        },
        "zoom" => TransitionKindIR::Zoom {
            origin: parse_vec2(t.params.get("origin"))
                .unwrap_or(crate::foundation::core::Vec2::new(0.5, 0.5)),
            from_scale: t
                .params
                .get("from_scale")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.9)
                .clamp(0.0, 1000.0) as f32,
        },
        "iris" => TransitionKindIR::Iris {
            origin: parse_vec2(t.params.get("origin"))
                .unwrap_or(crate::foundation::core::Vec2::new(0.5, 0.5)),
            shape: parse_iris_shape(t.params.get("shape").and_then(|v| v.as_str())),
            soft_edge: parse_soft_edge(t.params.get("soft_edge").and_then(|v| v.as_f64())),
        },
        _ => TransitionKindIR::Crossfade,
    };
    TransitionSpecIR {
        kind,
        duration_frames: t.duration_frames,
        ease,
    }
}

fn parse_wipe_dir(s: Option<&str>) -> WipeDirIR {
    match s.map(|x| x.trim().to_ascii_lowercase()) {
        None => WipeDirIR::LeftToRight,
        Some(s) => match s.as_str() {
            "left_to_right" | "lefttoright" | "ltr" => WipeDirIR::LeftToRight,
            "right_to_left" | "righttoleft" | "rtl" => WipeDirIR::RightToLeft,
            "top_to_bottom" | "toptobottom" | "ttb" => WipeDirIR::TopToBottom,
            "bottom_to_top" | "bottomtotop" | "btt" => WipeDirIR::BottomToTop,
            _ => WipeDirIR::LeftToRight,
        },
    }
}

fn parse_slide_dir(s: Option<&str>) -> SlideDirIR {
    match s.map(|x| x.trim().to_ascii_lowercase()) {
        None => SlideDirIR::Left,
        Some(s) => match s.as_str() {
            "left" => SlideDirIR::Left,
            "right" => SlideDirIR::Right,
            "up" => SlideDirIR::Up,
            "down" => SlideDirIR::Down,
            _ => SlideDirIR::Left,
        },
    }
}

fn parse_iris_shape(s: Option<&str>) -> IrisShapeIR {
    match s.map(|x| x.trim().to_ascii_lowercase()) {
        None => IrisShapeIR::Circle,
        Some(s) => match s.as_str() {
            "circle" => IrisShapeIR::Circle,
            "rect" | "rectangle" => IrisShapeIR::Rect,
            "diamond" => IrisShapeIR::Diamond,
            _ => IrisShapeIR::Circle,
        },
    }
}

fn parse_soft_edge(v: Option<f64>) -> f32 {
    match v {
        None => 0.0,
        Some(v) => {
            let f = v as f32;
            if !f.is_finite() {
                0.0
            } else {
                f.clamp(0.0, 1.0)
            }
        }
    }
}

fn parse_vec2(v: Option<&serde_json::Value>) -> Option<crate::foundation::core::Vec2> {
    let v = v?;
    if let Some(arr) = v.as_array()
        && arr.len() == 2
    {
        let x = arr[0].as_f64()?;
        let y = arr[1].as_f64()?;
        return Some(crate::foundation::core::Vec2::new(x, y));
    }
    if let Some(obj) = v.as_object() {
        let x = obj.get("x")?.as_f64()?;
        let y = obj.get("y")?.as_f64()?;
        return Some(crate::foundation::core::Vec2::new(x, y));
    }
    None
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

fn normalize_props(
    self_idx: NodeIdx,
    transform: TransformDef,
    opacity: &AnimDef<f64>,
    interner: &mut StringInterner,
    expr_sources: &mut Vec<ExprSourceIR>,
) -> Result<NodePropsIR, NormalizeError> {
    let opacity = normalize_lane_f64(
        self_idx,
        PropertyKey::Opacity,
        opacity,
        interner,
        expr_sources,
    )?;

    let translate_x = normalize_lane_f64(
        self_idx,
        PropertyKey::TransformTranslateX,
        &transform.translate.x,
        interner,
        expr_sources,
    )?;
    let translate_y = normalize_lane_f64(
        self_idx,
        PropertyKey::TransformTranslateY,
        &transform.translate.y,
        interner,
        expr_sources,
    )?;

    let rotation_rad = normalize_lane_f64_deg_to_rad(
        self_idx,
        PropertyKey::TransformRotationRad,
        &transform.rotation_deg,
        interner,
        expr_sources,
    )?;

    let scale_x = normalize_lane_f64(
        self_idx,
        PropertyKey::TransformScaleX,
        &transform.scale.x,
        interner,
        expr_sources,
    )?;
    let scale_y = normalize_lane_f64(
        self_idx,
        PropertyKey::TransformScaleY,
        &transform.scale.y,
        interner,
        expr_sources,
    )?;

    let anchor_x = normalize_lane_f64(
        self_idx,
        PropertyKey::TransformAnchorX,
        &transform.anchor.x,
        interner,
        expr_sources,
    )?;
    let anchor_y = normalize_lane_f64(
        self_idx,
        PropertyKey::TransformAnchorY,
        &transform.anchor.y,
        interner,
        expr_sources,
    )?;

    let skew_x_deg = normalize_lane_f64(
        self_idx,
        PropertyKey::TransformSkewX,
        &transform.skew_deg.x,
        interner,
        expr_sources,
    )?;
    let skew_y_deg = normalize_lane_f64(
        self_idx,
        PropertyKey::TransformSkewY,
        &transform.skew_deg.y,
        interner,
        expr_sources,
    )?;

    Ok(NodePropsIR {
        opacity,
        translate_x,
        translate_y,
        rotation_rad,
        scale_x,
        scale_y,
        anchor_x,
        anchor_y,
        skew_x_deg,
        skew_y_deg,
        switch_active: None,
    })
}

fn normalize_layout(
    self_idx: NodeIdx,
    layout: Option<&LayoutPropsDef>,
    interner: &mut StringInterner,
    expr_sources: &mut Vec<ExprSourceIR>,
) -> Result<Option<LayoutPropsIR>, NormalizeError> {
    let Some(layout) = layout else {
        return Ok(None);
    };

    let display = match layout.display {
        LayoutDisplayDef::None => LayoutDisplayIR::None,
        LayoutDisplayDef::Flex => LayoutDisplayIR::Flex,
        LayoutDisplayDef::Grid => LayoutDisplayIR::Grid,
    };
    let direction = match layout.direction {
        LayoutDirectionDef::Row => LayoutDirectionIR::Row,
        LayoutDirectionDef::Column => LayoutDirectionIR::Column,
    };
    let wrap = match layout.wrap {
        LayoutWrapDef::NoWrap => LayoutWrapIR::NoWrap,
        LayoutWrapDef::Wrap => LayoutWrapIR::Wrap,
    };
    let justify_content = match layout.justify_content {
        LayoutJustifyContentDef::Start => LayoutJustifyContentIR::Start,
        LayoutJustifyContentDef::End => LayoutJustifyContentIR::End,
        LayoutJustifyContentDef::Center => LayoutJustifyContentIR::Center,
        LayoutJustifyContentDef::SpaceBetween => LayoutJustifyContentIR::SpaceBetween,
        LayoutJustifyContentDef::SpaceAround => LayoutJustifyContentIR::SpaceAround,
        LayoutJustifyContentDef::SpaceEvenly => LayoutJustifyContentIR::SpaceEvenly,
    };
    let align_items = match layout.align_items {
        LayoutAlignItemsDef::Start => LayoutAlignItemsIR::Start,
        LayoutAlignItemsDef::End => LayoutAlignItemsIR::End,
        LayoutAlignItemsDef::Center => LayoutAlignItemsIR::Center,
        LayoutAlignItemsDef::Stretch => LayoutAlignItemsIR::Stretch,
    };
    let align_content = match layout.align_content {
        LayoutAlignContentDef::Start => LayoutAlignContentIR::Start,
        LayoutAlignContentDef::End => LayoutAlignContentIR::End,
        LayoutAlignContentDef::Center => LayoutAlignContentIR::Center,
        LayoutAlignContentDef::SpaceBetween => LayoutAlignContentIR::SpaceBetween,
        LayoutAlignContentDef::SpaceAround => LayoutAlignContentIR::SpaceAround,
        LayoutAlignContentDef::SpaceEvenly => LayoutAlignContentIR::SpaceEvenly,
        LayoutAlignContentDef::Stretch => LayoutAlignContentIR::Stretch,
    };
    let position = match layout.position {
        LayoutPositionDef::Relative => LayoutPositionIR::Relative,
        LayoutPositionDef::Absolute => LayoutPositionIR::Absolute,
    };

    let gap_x_px = normalize_lane_f64(
        self_idx,
        PropertyKey::LayoutGapX,
        &layout.gap_px.x,
        interner,
        expr_sources,
    )?;
    let gap_y_px = normalize_lane_f64(
        self_idx,
        PropertyKey::LayoutGapY,
        &layout.gap_px.y,
        interner,
        expr_sources,
    )?;

    let padding_px = normalize_edges_anim(
        self_idx,
        &layout.padding_px,
        (
            PropertyKey::LayoutPaddingTopPx,
            PropertyKey::LayoutPaddingRightPx,
            PropertyKey::LayoutPaddingBottomPx,
            PropertyKey::LayoutPaddingLeftPx,
        ),
        interner,
        expr_sources,
    )?;
    let margin_px = normalize_edges_anim(
        self_idx,
        &layout.margin_px,
        (
            PropertyKey::LayoutMarginTopPx,
            PropertyKey::LayoutMarginRightPx,
            PropertyKey::LayoutMarginBottomPx,
            PropertyKey::LayoutMarginLeftPx,
        ),
        interner,
        expr_sources,
    )?;

    let flex_grow = normalize_lane_f64(
        self_idx,
        PropertyKey::LayoutFlexGrow,
        &layout.flex_grow,
        interner,
        expr_sources,
    )?;
    let flex_shrink = normalize_lane_f64(
        self_idx,
        PropertyKey::LayoutFlexShrink,
        &layout.flex_shrink,
        interner,
        expr_sources,
    )?;

    let size = normalize_size(
        self_idx,
        &layout.size,
        (PropertyKey::LayoutWidthPx, PropertyKey::LayoutHeightPx),
        interner,
        expr_sources,
    )?;
    let min_size = normalize_size(
        self_idx,
        &layout.min_size,
        (
            PropertyKey::LayoutMinWidthPx,
            PropertyKey::LayoutMinHeightPx,
        ),
        interner,
        expr_sources,
    )?;
    let max_size = normalize_size(
        self_idx,
        &layout.max_size,
        (
            PropertyKey::LayoutMaxWidthPx,
            PropertyKey::LayoutMaxHeightPx,
        ),
        interner,
        expr_sources,
    )?;

    Ok(Some(LayoutPropsIR {
        display,
        direction,
        wrap,
        justify_content,
        align_items,
        align_content,
        position,
        gap_x_px,
        gap_y_px,
        padding_px,
        margin_px,
        flex_grow,
        flex_shrink,
        size,
        min_size,
        max_size,
    }))
}

fn normalize_edges_anim(
    self_idx: NodeIdx,
    e: &EdgesAnimDef,
    keys: (PropertyKey, PropertyKey, PropertyKey, PropertyKey),
    interner: &mut StringInterner,
    expr_sources: &mut Vec<ExprSourceIR>,
) -> Result<EdgesAnimIR, NormalizeError> {
    Ok(EdgesAnimIR {
        top: normalize_lane_f64(self_idx, keys.0, &e.top, interner, expr_sources)?,
        right: normalize_lane_f64(self_idx, keys.1, &e.right, interner, expr_sources)?,
        bottom: normalize_lane_f64(self_idx, keys.2, &e.bottom, interner, expr_sources)?,
        left: normalize_lane_f64(self_idx, keys.3, &e.left, interner, expr_sources)?,
    })
}

fn normalize_size(
    self_idx: NodeIdx,
    s: &SizeDef,
    keys: (PropertyKey, PropertyKey),
    interner: &mut StringInterner,
    expr_sources: &mut Vec<ExprSourceIR>,
) -> Result<SizeAnimIR, NormalizeError> {
    Ok(SizeAnimIR {
        width: normalize_dimension(self_idx, &s.width, keys.0, interner, expr_sources)?,
        height: normalize_dimension(self_idx, &s.height, keys.1, interner, expr_sources)?,
    })
}

fn normalize_dimension(
    self_idx: NodeIdx,
    d: &AnimDimensionDef,
    px_key: PropertyKey,
    interner: &mut StringInterner,
    expr_sources: &mut Vec<ExprSourceIR>,
) -> Result<AnimDimensionIR, NormalizeError> {
    match d {
        AnimDimensionDef::Auto => Ok(AnimDimensionIR::Auto),
        AnimDimensionDef::Percent(p) => Ok(AnimDimensionIR::Percent((*p as f32).max(0.0))),
        AnimDimensionDef::Px(px) => Ok(AnimDimensionIR::Px(normalize_lane_f64(
            self_idx,
            px_key,
            px,
            interner,
            expr_sources,
        )?)),
    }
}

fn normalize_lane_f64(
    self_idx: NodeIdx,
    key: PropertyKey,
    a: &AnimDef<f64>,
    interner: &mut StringInterner,
    expr_sources: &mut Vec<ExprSourceIR>,
) -> Result<Anim<f64>, NormalizeError> {
    normalize_lane_f64_scaled(self_idx, key, a, 1.0, interner, expr_sources)
}

fn normalize_lane_f64_deg_to_rad(
    self_idx: NodeIdx,
    key: PropertyKey,
    a: &AnimDef<f64>,
    interner: &mut StringInterner,
    expr_sources: &mut Vec<ExprSourceIR>,
) -> Result<Anim<f64>, NormalizeError> {
    const DEG_TO_RAD: f64 = std::f64::consts::PI / 180.0;
    match a {
        AnimDef::Constant(v) => Ok(Anim::Constant(v * DEG_TO_RAD)),
        AnimDef::Tagged(tag) => match tag {
            AnimTaggedDef::Keyframes(k) => Ok(Anim::Keyframes(Keyframes {
                keys: k
                    .keys
                    .iter()
                    .map(|kk| Keyframe {
                        frame: kk.frame,
                        value: kk.value * DEG_TO_RAD,
                    })
                    .collect(),
                mode: InterpMode::from(k.mode),
                default: k.default.map(|v| v * DEG_TO_RAD),
            })),
            AnimTaggedDef::Procedural(p) => Ok(Anim::Procedural(Procedural::new(
                ProceduralKind::from(scale_procedural(p.clone(), DEG_TO_RAD)),
            ))),
            AnimTaggedDef::Expr(s) => {
                let pid = PropertyIndex::property_id(self_idx, key);
                let wrapped = wrap_scale_expr(s, DEG_TO_RAD);
                let src = interner.intern(&wrapped);
                expr_sources.push(ExprSourceIR {
                    target: pid,
                    value_type: ValueTypeIR::F64,
                    src,
                });
                Ok(Anim::Reference(pid))
            }
        },
    }
}

fn normalize_lane_f64_scaled(
    self_idx: NodeIdx,
    key: PropertyKey,
    a: &AnimDef<f64>,
    scale: f64,
    interner: &mut StringInterner,
    expr_sources: &mut Vec<ExprSourceIR>,
) -> Result<Anim<f64>, NormalizeError> {
    match a {
        AnimDef::Constant(v) => Ok(Anim::Constant(v * scale)),
        AnimDef::Tagged(tag) => match tag {
            AnimTaggedDef::Keyframes(k) => Ok(Anim::Keyframes(Keyframes {
                keys: k
                    .keys
                    .iter()
                    .map(|kk| Keyframe {
                        frame: kk.frame,
                        value: kk.value * scale,
                    })
                    .collect(),
                mode: InterpMode::from(k.mode),
                default: k.default.map(|v| v * scale),
            })),
            AnimTaggedDef::Procedural(p) => Ok(Anim::Procedural(Procedural::new(
                ProceduralKind::from(scale_procedural(p.clone(), scale)),
            ))),
            AnimTaggedDef::Expr(s) => {
                let pid = PropertyIndex::property_id(self_idx, key);
                let src = interner.intern(s);
                expr_sources.push(ExprSourceIR {
                    target: pid,
                    value_type: ValueTypeIR::F64,
                    src,
                });
                Ok(Anim::Reference(pid))
            }
        },
    }
}

fn normalize_lane_u64(
    self_idx: NodeIdx,
    key: PropertyKey,
    a: &AnimDef<u64>,
    interner: &mut StringInterner,
    expr_sources: &mut Vec<ExprSourceIR>,
) -> Result<Anim<u64>, NormalizeError> {
    match a {
        AnimDef::Constant(v) => Ok(Anim::Constant(*v)),
        AnimDef::Tagged(tag) => match tag {
            AnimTaggedDef::Keyframes(k) => Ok(Anim::Keyframes(Keyframes {
                keys: k
                    .keys
                    .iter()
                    .map(|kk| Keyframe {
                        frame: kk.frame,
                        value: kk.value,
                    })
                    .collect(),
                mode: InterpMode::from(k.mode),
                default: k.default,
            })),
            AnimTaggedDef::Procedural(_p) => Ok(Anim::Constant(0)),
            AnimTaggedDef::Expr(s) => {
                let pid = PropertyIndex::property_id(self_idx, key);
                let src = interner.intern(s);
                expr_sources.push(ExprSourceIR {
                    target: pid,
                    value_type: ValueTypeIR::U64,
                    src,
                });
                Ok(Anim::Reference(pid))
            }
        },
    }
}

fn wrap_scale_expr(expr: &str, scale: f64) -> String {
    // Normalize to a single `=<expr>` form.
    let e = expr.trim();
    let inner = e.strip_prefix('=').unwrap_or(e);
    format!("=({inner})*{scale}")
}

fn scale_procedural(
    p: crate::animation::anim::ProceduralDef,
    scale: f64,
) -> crate::animation::anim::ProceduralDef {
    use crate::animation::anim::{ProcScalarDef, ProceduralDef};
    fn scale_scalar(s: ProcScalarDef, scale: f64) -> ProcScalarDef {
        match s {
            ProcScalarDef::Sine {
                amp,
                freq_hz,
                phase,
                offset,
            } => ProcScalarDef::Sine {
                amp: amp * scale,
                freq_hz,
                phase,
                offset: offset * scale,
            },
            ProcScalarDef::Noise1d {
                amp,
                freq_hz,
                offset,
            } => ProcScalarDef::Noise1d {
                amp: amp * scale,
                freq_hz,
                offset: offset * scale,
            },
        }
    }

    match p {
        ProceduralDef::Scalar(s) => ProceduralDef::Scalar(scale_scalar(s, scale)),
        ProceduralDef::Vec2 { x, y } => ProceduralDef::Vec2 {
            x: scale_scalar(x, scale),
            y: scale_scalar(y, scale),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::anim::AnimDef;
    use crate::scene::model::{CanvasDef, FpsDef, NodeKindDef, TransformDef};
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
                        opacity: AnimDef::Constant(1.0),
                        blend: Default::default(),
                        layout: None,
                        effects: vec![],
                        mask: None,
                        transition_in: None,
                        transition_out: None,
                    }],
                },
                range: [0, 10],
                transform: TransformDef::default(),
                opacity: AnimDef::Constant(1.0),
                blend: Default::default(),
                layout: None,
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
