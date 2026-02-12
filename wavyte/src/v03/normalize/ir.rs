use crate::foundation::core::Rgba8Premul;
use crate::foundation::core::{Canvas, Fps};
use crate::v03::animation::anim::Anim;
use crate::v03::animation::anim::InterpMode;
use crate::v03::effects::binding::EffectBindingIR;
use crate::v03::foundation::ids::{AssetIdx, NodeIdx, VarId};
use crate::v03::normalize::intern::{InternId, StringInterner};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::ops::Range;

#[derive(Debug)]
pub(crate) struct NormalizedComposition {
    pub(crate) ir: CompositionIR,
    pub(crate) interner: StringInterner,
    pub(crate) expr_sources: Vec<ExprSourceIR>,

    pub(crate) node_id_by_idx: Vec<InternId>,
    pub(crate) node_idx_by_id: HashMap<InternId, NodeIdx>,

    pub(crate) asset_key_by_idx: Vec<InternId>,
    pub(crate) asset_idx_by_key: HashMap<InternId, AssetIdx>,

    pub(crate) var_key_by_id: Vec<InternId>,
    pub(crate) var_id_by_key: HashMap<InternId, VarId>,
}

#[derive(Debug, Clone)]
pub(crate) struct CompositionIR {
    pub(crate) canvas: Canvas,
    pub(crate) fps: Fps,
    pub(crate) duration_frames: u64,
    pub(crate) seed: u64,

    pub(crate) vars: Vec<VarValueIR>,
    pub(crate) assets: Vec<AssetIR>,

    pub(crate) nodes: Vec<NodeIR>,
    pub(crate) root: NodeIdx,

    pub(crate) layout: LayoutIR,
    pub(crate) registries: RegistryBindings,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LayoutIR {
    // Phase 4 wires this into the Taffy bridge. Kept as a placeholder so `CompositionIR`
    // doesn’t need a structural redesign once layout is introduced.
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RegistryBindings {
    pub(crate) effect_intern_by_id: Vec<InternId>,
    /// Dense map: `InternId.0 as usize -> Some(EffectKindId)`.
    pub(crate) effect_kind_by_intern: Vec<Option<crate::v03::foundation::ids::EffectKindId>>,

    pub(crate) param_intern_by_id: Vec<InternId>,
    /// Dense map: `InternId.0 as usize -> Some(ParamId)`.
    pub(crate) param_id_by_intern: Vec<Option<crate::v03::foundation::ids::ParamId>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueTypeIR {
    Bool,
    F64,
    U64,
    Color,
}

#[derive(Debug, Clone)]
pub(crate) struct ExprSourceIR {
    pub(crate) target: crate::v03::foundation::ids::PropertyId,
    pub(crate) value_type: ValueTypeIR,
    pub(crate) src: InternId,
}

#[derive(Debug, Clone)]
pub(crate) enum VarValueIR {
    Bool(bool),
    F64(f64),
    Vec2 { x: f64, y: f64 },
    Color(Rgba8Premul),
}

#[derive(Debug, Clone)]
pub(crate) enum AssetIR {
    Image {
        source: InternId,
    },
    Svg {
        source: InternId,
    },
    Path {
        svg_path_d: InternId,
    },
    Text {
        text: InternId,
        font_source: InternId,
        size_px: f64,
        max_width_px: Option<f64>,
        color: Option<VarValueIR>,
    },
    Video {
        source: InternId,
        trim_start_sec: f64,
        trim_end_sec: Option<f64>,
        playback_rate: f64,
    },
    Audio {
        source: InternId,
        trim_start_sec: f64,
        trim_end_sec: Option<f64>,
        playback_rate: f64,
    },
    Null,
}

#[derive(Debug, Clone)]
pub(crate) struct NodeIR {
    pub(crate) id: InternId,
    pub(crate) range: Range<u64>,
    pub(crate) kind: NodeKindIR,

    pub(crate) props: NodePropsIR,
    pub(crate) layout: Option<LayoutPropsIR>,

    pub(crate) effects: Vec<EffectBindingIR>,
    pub(crate) mask: Option<MaskIR>,
    pub(crate) transition_in: Option<TransitionSpecIR>,
    pub(crate) transition_out: Option<TransitionSpecIR>,
}

#[derive(Debug, Clone)]
pub(crate) struct NodePropsIR {
    pub(crate) opacity: Anim<f64>,
    pub(crate) translate_x: Anim<f64>,
    pub(crate) translate_y: Anim<f64>,
    pub(crate) rotation_rad: Anim<f64>,
    pub(crate) scale_x: Anim<f64>,
    pub(crate) scale_y: Anim<f64>,
    pub(crate) anchor_x: Anim<f64>,
    pub(crate) anchor_y: Anim<f64>,
    pub(crate) skew_x_deg: Anim<f64>,
    pub(crate) skew_y_deg: Anim<f64>,

    pub(crate) switch_active: Option<Anim<u64>>,
}

#[derive(Debug, Clone)]
pub(crate) struct LayoutPropsIR {
    pub(crate) display: LayoutDisplayIR,
    pub(crate) direction: LayoutDirectionIR,
    pub(crate) wrap: LayoutWrapIR,
    pub(crate) justify_content: LayoutJustifyContentIR,
    pub(crate) align_items: LayoutAlignItemsIR,
    pub(crate) align_content: LayoutAlignContentIR,
    pub(crate) position: LayoutPositionIR,

    pub(crate) gap_x_px: Anim<f64>,
    pub(crate) gap_y_px: Anim<f64>,
    pub(crate) padding_px: EdgesAnimIR,
    pub(crate) margin_px: EdgesAnimIR,

    pub(crate) flex_grow: Anim<f64>,
    pub(crate) flex_shrink: Anim<f64>,

    pub(crate) size: SizeAnimIR,
    pub(crate) min_size: SizeAnimIR,
    pub(crate) max_size: SizeAnimIR,
}

#[derive(Debug, Clone)]
pub(crate) struct EdgesAnimIR {
    pub(crate) top: Anim<f64>,
    pub(crate) right: Anim<f64>,
    pub(crate) bottom: Anim<f64>,
    pub(crate) left: Anim<f64>,
}

#[derive(Debug, Clone)]
pub(crate) struct SizeAnimIR {
    pub(crate) width: AnimDimensionIR,
    pub(crate) height: AnimDimensionIR,
}

#[derive(Debug, Clone)]
pub(crate) enum AnimDimensionIR {
    Auto,
    Px(Anim<f64>),
    Percent(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutDisplayIR {
    None,
    Flex,
    Grid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutDirectionIR {
    Row,
    Column,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutWrapIR {
    NoWrap,
    Wrap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutJustifyContentIR {
    Start,
    End,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutAlignItemsIR {
    Start,
    End,
    Center,
    Stretch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutAlignContentIR {
    Start,
    End,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
    Stretch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutPositionIR {
    Relative,
    Absolute,
}

#[derive(Debug, Clone)]
pub(crate) enum NodeKindIR {
    Leaf {
        asset: AssetIdx,
    },
    Collection {
        mode: CollectionModeIR,
        children: Vec<NodeIdx>,
        /// Only present for `Sequence`.
        sequence_prefix_starts: Option<Vec<u64>>,
    },
    CompRef {
        composition: JsonValue,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum CollectionModeIR {
    Group,
    Stack,
    Sequence,
    Switch,
}

#[derive(Debug, Clone)]
pub(crate) enum MaskSourceIR {
    Node(InternId),
    Asset(InternId),
    Shape(ShapeIR),
}

#[derive(Debug, Clone)]
pub(crate) struct MaskIR {
    pub(crate) source: MaskSourceIR,
    pub(crate) mode: MaskModeIR,
    pub(crate) inverted: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum ShapeIR {
    Rect {
        width: f64,
        height: f64,
    },
    RoundedRect {
        width: f64,
        height: f64,
        radius: f64,
    },
    Ellipse {
        rx: f64,
        ry: f64,
    },
    Path {
        svg_path_d: InternId,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum MaskModeIR {
    Alpha,
    Luma,
    Stencil { threshold: f32 },
}

#[derive(Debug, Clone)]
pub(crate) struct TransitionSpecIR {
    pub(crate) kind: InternId,
    pub(crate) duration_frames: u32,
    pub(crate) ease: InterpMode,
    pub(crate) params: Vec<(InternId, JsonValue)>,
}

// Effects are bound during normalization to avoid any runtime param-name lookups.
