use crate::animation::anim::AnimDef;
use crate::assets::color::ColorDef;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct CanvasDef {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct FpsDef {
    pub(crate) num: u32,
    pub(crate) den: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub(crate) struct Vec2Def {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

impl<'de> Deserialize<'de> for Vec2Def {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Arr([f64; 2]),
            Obj { x: f64, y: f64 },
        }

        match Repr::deserialize(deserializer)? {
            Repr::Arr([x, y]) => Ok(Self { x, y }),
            Repr::Obj { x, y } => Ok(Self { x, y }),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub(crate) struct EdgesDef {
    pub(crate) top: f64,
    pub(crate) right: f64,
    pub(crate) bottom: f64,
    pub(crate) left: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TransformDef {
    pub(crate) translate: Vec2AnimDef,
    pub(crate) rotation_deg: AnimDef<f64>,
    pub(crate) scale: Vec2AnimDef,
    pub(crate) anchor: Vec2AnimDef,
    pub(crate) skew_deg: Vec2AnimDef,
}

impl Default for TransformDef {
    fn default() -> Self {
        Self {
            translate: Vec2AnimDef::constant(0.0, 0.0),
            rotation_deg: AnimDef::Constant(0.0),
            scale: Vec2AnimDef::constant(1.0, 1.0),
            anchor: Vec2AnimDef::constant(0.0, 0.0),
            skew_deg: Vec2AnimDef::constant(0.0, 0.0),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Vec2AnimDef {
    pub(crate) x: AnimDef<f64>,
    pub(crate) y: AnimDef<f64>,
}

impl Vec2AnimDef {
    pub(crate) fn constant(x: f64, y: f64) -> Self {
        Self {
            x: AnimDef::Constant(x),
            y: AnimDef::Constant(y),
        }
    }
}

impl Default for Vec2AnimDef {
    fn default() -> Self {
        Self::constant(0.0, 0.0)
    }
}

impl<'de> Deserialize<'de> for Vec2AnimDef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Arr([AnimDef<f64>; 2]),
            Obj { x: AnimDef<f64>, y: AnimDef<f64> },
        }

        match Repr::deserialize(deserializer)? {
            Repr::Arr([x, y]) => Ok(Self { x, y }),
            Repr::Obj { x, y } => Ok(Self { x, y }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CompositionDef {
    pub(crate) version: String,
    pub(crate) canvas: CanvasDef,
    pub(crate) fps: FpsDef,
    pub(crate) duration: u64,
    #[serde(default)]
    pub(crate) seed: u64,
    #[serde(default)]
    pub(crate) variables: BTreeMap<String, VarDef>,
    #[serde(default)]
    pub(crate) assets: BTreeMap<String, AssetDef>,
    pub(crate) root: NodeDef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum VarDef {
    Bool(bool),
    F64(f64),
    Vec2(Vec2Def),
    Color(ColorDef),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct NodeDef {
    pub(crate) id: String,
    pub(crate) kind: NodeKindDef,
    pub(crate) range: [u64; 2],

    #[serde(default)]
    pub(crate) transform: TransformDef,
    #[serde(default = "default_opacity")]
    pub(crate) opacity: AnimDef<f64>,
    #[serde(default)]
    pub(crate) blend: BlendModeDef,

    /// Optional layout participation (Flex/Grid) for this node.
    ///
    /// v0.3 keeps a performance-focused subset; see `wavyte_v03_proposal_final.md` section 10.
    #[serde(default)]
    pub(crate) layout: Option<LayoutPropsDef>,

    #[serde(default)]
    pub(crate) effects: Vec<EffectInstanceDef>,
    #[serde(default)]
    pub(crate) mask: Option<MaskDef>,
    #[serde(default)]
    pub(crate) transition_in: Option<TransitionSpecDef>,
    #[serde(default)]
    pub(crate) transition_out: Option<TransitionSpecDef>,
}

fn default_opacity() -> AnimDef<f64> {
    AnimDef::Constant(1.0)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BlendModeDef {
    #[default]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NodeKindDef {
    Leaf {
        asset: String,
    },
    Collection {
        mode: CollectionModeDef,
        children: Vec<NodeDef>,
    },
    CompRef {
        composition: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CollectionModeDef {
    Group,
    Sequence,
    Stack,
    Switch { active: AnimDef<u64> },
}

// ----------------------------
// Layout (boundary)
// ----------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LayoutPropsDef {
    #[serde(default)]
    pub(crate) display: LayoutDisplayDef,
    #[serde(default)]
    pub(crate) direction: LayoutDirectionDef,
    #[serde(default)]
    pub(crate) wrap: LayoutWrapDef,
    #[serde(default)]
    pub(crate) justify_content: LayoutJustifyContentDef,
    #[serde(default)]
    pub(crate) align_items: LayoutAlignItemsDef,
    #[serde(default)]
    pub(crate) align_content: LayoutAlignContentDef,
    #[serde(default)]
    pub(crate) position: LayoutPositionDef,

    #[serde(default)]
    pub(crate) gap_px: Vec2AnimDef,
    #[serde(default)]
    pub(crate) padding_px: EdgesAnimDef,
    #[serde(default)]
    pub(crate) margin_px: EdgesAnimDef,

    #[serde(default = "default_flex_grow")]
    pub(crate) flex_grow: AnimDef<f64>,
    #[serde(default = "default_flex_shrink")]
    pub(crate) flex_shrink: AnimDef<f64>,

    #[serde(default)]
    pub(crate) size: SizeDef,
    #[serde(default)]
    pub(crate) min_size: SizeDef,
    #[serde(default)]
    pub(crate) max_size: SizeDef,
}

fn default_flex_grow() -> AnimDef<f64> {
    AnimDef::Constant(0.0)
}

fn default_flex_shrink() -> AnimDef<f64> {
    AnimDef::Constant(1.0)
}

impl Default for LayoutPropsDef {
    fn default() -> Self {
        Self {
            display: LayoutDisplayDef::default(),
            direction: LayoutDirectionDef::default(),
            wrap: LayoutWrapDef::default(),
            justify_content: LayoutJustifyContentDef::default(),
            align_items: LayoutAlignItemsDef::default(),
            align_content: LayoutAlignContentDef::default(),
            position: LayoutPositionDef::default(),
            gap_px: Vec2AnimDef::default(),
            padding_px: EdgesAnimDef::default(),
            margin_px: EdgesAnimDef::default(),
            flex_grow: default_flex_grow(),
            flex_shrink: default_flex_shrink(),
            size: SizeDef::default(),
            min_size: SizeDef::default(),
            max_size: SizeDef::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LayoutDisplayDef {
    None,
    #[default]
    Flex,
    Grid,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LayoutDirectionDef {
    #[default]
    Row,
    Column,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LayoutWrapDef {
    #[default]
    NoWrap,
    Wrap,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LayoutJustifyContentDef {
    #[default]
    Start,
    End,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LayoutAlignItemsDef {
    Start,
    End,
    Center,
    #[default]
    Stretch,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LayoutAlignContentDef {
    Start,
    End,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
    #[default]
    Stretch,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LayoutPositionDef {
    #[default]
    Relative,
    Absolute,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct EdgesAnimDef {
    pub(crate) top: AnimDef<f64>,
    pub(crate) right: AnimDef<f64>,
    pub(crate) bottom: AnimDef<f64>,
    pub(crate) left: AnimDef<f64>,
}

impl Default for EdgesAnimDef {
    fn default() -> Self {
        Self {
            top: AnimDef::Constant(0.0),
            right: AnimDef::Constant(0.0),
            bottom: AnimDef::Constant(0.0),
            left: AnimDef::Constant(0.0),
        }
    }
}

impl<'de> Deserialize<'de> for EdgesAnimDef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct EdgesObj {
            top: Option<AnimDef<f64>>,
            right: Option<AnimDef<f64>>,
            bottom: Option<AnimDef<f64>>,
            left: Option<AnimDef<f64>>,
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Obj(Box<EdgesObj>),
            // Shorthand: single number applies to all edges.
            Num(f64),
        }

        match Repr::deserialize(deserializer)? {
            Repr::Num(v) => Ok(Self {
                top: AnimDef::Constant(v),
                right: AnimDef::Constant(v),
                bottom: AnimDef::Constant(v),
                left: AnimDef::Constant(v),
            }),
            Repr::Obj(obj) => {
                let d = Self::default();
                Ok(Self {
                    top: obj.top.unwrap_or(d.top),
                    right: obj.right.unwrap_or(d.right),
                    bottom: obj.bottom.unwrap_or(d.bottom),
                    left: obj.left.unwrap_or(d.left),
                })
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct SizeDef {
    #[serde(default)]
    pub(crate) width: AnimDimensionDef,
    #[serde(default)]
    pub(crate) height: AnimDimensionDef,
}

#[derive(Debug, Clone, Default)]
pub(crate) enum AnimDimensionDef {
    #[default]
    Auto,
    Px(AnimDef<f64>),
    Percent(f64),
}

impl<'de> Deserialize<'de> for AnimDimensionDef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Str(String),
            Num(f64),
            ObjPx { px: AnimDef<f64> },
            ObjPercent { percent: f64 },
        }

        match Repr::deserialize(deserializer)? {
            Repr::Str(s) => match s.as_str() {
                "auto" => Ok(Self::Auto),
                other => Err(serde::de::Error::custom(format!(
                    "unknown dimension string \"{other}\" (expected \"auto\")"
                ))),
            },
            Repr::Num(v) => Ok(Self::Px(AnimDef::Constant(v))),
            Repr::ObjPx { px } => Ok(Self::Px(px)),
            Repr::ObjPercent { percent } => Ok(Self::Percent(percent)),
        }
    }
}

impl serde::Serialize for AnimDimensionDef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        match self {
            Self::Auto => serializer.serialize_str("auto"),
            Self::Px(px) => {
                let mut m = serializer.serialize_map(Some(1))?;
                m.serialize_entry("px", px)?;
                m.end()
            }
            Self::Percent(p) => {
                let mut m = serializer.serialize_map(Some(1))?;
                m.serialize_entry("percent", p)?;
                m.end()
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MaskDef {
    pub(crate) source: MaskSourceDef,
    #[serde(default)]
    pub(crate) mode: MaskModeDef,
    #[serde(default)]
    pub(crate) inverted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MaskSourceDef {
    Node(String),
    Asset(String),
    Shape(ShapeDef),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ShapeDef {
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
        svg_path_d: String,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MaskModeDef {
    #[default]
    Alpha,
    Luma,
    Stencil {
        threshold: f32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TransitionSpecDef {
    pub(crate) kind: String,
    pub(crate) duration_frames: u32,
    #[serde(default)]
    pub(crate) ease: Option<String>,
    #[serde(default)]
    pub(crate) params: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct EffectInstanceDef {
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) params: BTreeMap<String, serde_json::Value>,
}

impl<'de> Deserialize<'de> for EffectInstanceDef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Full {
                kind: String,
                #[serde(default)]
                params: BTreeMap<String, serde_json::Value>,
            },
            Shorthand(BTreeMap<String, serde_json::Value>),
        }

        match Repr::deserialize(deserializer)? {
            Repr::Full { kind, params } => Ok(Self { kind, params }),
            Repr::Shorthand(map) => {
                if map.len() != 1 {
                    return Err(serde::de::Error::custom(
                        "effect shorthand must be an object with exactly one key",
                    ));
                }
                let (kind, v) = map.into_iter().next().unwrap();
                let params = match v {
                    serde_json::Value::Null => BTreeMap::new(),
                    serde_json::Value::Object(obj) => obj.into_iter().collect(),
                    other => {
                        // Single scalar shorthand: treat as {"value": other}.
                        let mut m = BTreeMap::new();
                        m.insert("value".to_owned(), other);
                        m
                    }
                };
                Ok(Self { kind, params })
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AssetDef {
    Image {
        source: String,
    },
    Svg {
        source: String,
    },
    Path {
        svg_path_d: String,
    },
    Text {
        text: String,
        font_source: String,
        size_px: f64,
        #[serde(default)]
        max_width_px: Option<f64>,
        #[serde(default)]
        color: Option<ColorDef>,
    },
    Video {
        source: String,
        #[serde(default)]
        trim_start_sec: f64,
        #[serde(default)]
        trim_end_sec: Option<f64>,
        #[serde(default = "default_playback_rate")]
        playback_rate: f64,
        #[serde(default = "default_volume")]
        volume: f64,
        #[serde(default)]
        mute: bool,
        #[serde(default)]
        fade_in_sec: f64,
        #[serde(default)]
        fade_out_sec: f64,
    },
    Audio {
        source: String,
        #[serde(default)]
        trim_start_sec: f64,
        #[serde(default)]
        trim_end_sec: Option<f64>,
        #[serde(default = "default_playback_rate")]
        playback_rate: f64,
        #[serde(default = "default_volume")]
        volume: f64,
        #[serde(default)]
        mute: bool,
        #[serde(default)]
        fade_in_sec: f64,
        #[serde(default)]
        fade_out_sec: f64,
    },
    SolidRect {
        #[serde(default)]
        color: Option<ColorDef>,
    },
    Gradient {
        start: ColorDef,
        end: ColorDef,
    },
    Noise {
        #[serde(default)]
        seed: u64,
    },
    Null,
}

fn default_playback_rate() -> f64 {
    1.0
}

fn default_volume() -> f64 {
    1.0
}
