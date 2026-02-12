use crate::foundation::core::{Canvas, Fps};
use crate::v03::foundation::ids::{AssetIdx, NodeIdx, VarId};
use crate::v03::normalize::intern::{InternId, StringInterner};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::ops::Range;

#[derive(Debug)]
pub(crate) struct NormalizedComposition {
    pub(crate) ir: CompositionIR,
    pub(crate) interner: StringInterner,

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
    // Phase 3/5 bind effect and transition kinds/params to dense IDs.
}

#[derive(Debug, Clone)]
pub(crate) enum VarValueIR {
    Bool(bool),
    F64(f64),
    Vec2 { x: f64, y: f64 },
    HexColor(InternId),
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

    pub(crate) effects: Vec<EffectInstanceIR>,
    pub(crate) mask: Option<MaskIR>,
    pub(crate) transition_in: Option<TransitionSpecIR>,
    pub(crate) transition_out: Option<TransitionSpecIR>,
}

#[derive(Debug, Clone)]
pub(crate) struct NodePropsIR {
    pub(crate) opacity: AnimIR<f64>,
    pub(crate) translate: AnimIR<(f64, f64)>,
    pub(crate) rotation_deg: AnimIR<f64>,
    pub(crate) scale: AnimIR<(f64, f64)>,
    pub(crate) anchor: AnimIR<(f64, f64)>,
    pub(crate) skew_deg: AnimIR<(f64, f64)>,

    pub(crate) switch_active: Option<AnimIR<u64>>,
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
    pub(crate) ease: Option<InternId>,
    pub(crate) params: Vec<(InternId, JsonValue)>,
}

#[derive(Debug, Clone)]
pub(crate) struct EffectInstanceIR {
    pub(crate) kind: InternId,
    pub(crate) params: Vec<(InternId, JsonValue)>,
}

#[derive(Debug, Clone)]
pub(crate) enum AnimIR<T> {
    Constant(T),
    Keyframes(KeyframesIR<T>),
    Expr(InternId),
}

#[derive(Debug, Clone)]
pub(crate) struct KeyframesIR<T> {
    pub(crate) keys: Vec<KeyframeIR<T>>,
    pub(crate) mode: InterpModeIR,
}

#[derive(Debug, Clone)]
pub(crate) struct KeyframeIR<T> {
    pub(crate) frame: u64,
    pub(crate) value: T,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) enum InterpModeIR {
    #[default]
    Linear,
    Hold,
}
