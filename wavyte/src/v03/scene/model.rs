use crate::v03::animation::anim::Anim;
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
    pub(crate) translate: Anim<Vec2Def>,
    pub(crate) rotation_deg: Anim<f64>,
    pub(crate) scale: Anim<Vec2Def>,
    pub(crate) anchor: Anim<Vec2Def>,
    pub(crate) skew_deg: Anim<Vec2Def>,
}

impl Default for TransformDef {
    fn default() -> Self {
        Self {
            translate: Anim::Constant(Vec2Def { x: 0.0, y: 0.0 }),
            rotation_deg: Anim::Constant(0.0),
            scale: Anim::Constant(Vec2Def { x: 1.0, y: 1.0 }),
            anchor: Anim::Constant(Vec2Def { x: 0.0, y: 0.0 }),
            skew_deg: Anim::Constant(Vec2Def { x: 0.0, y: 0.0 }),
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
    HexColor(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct NodeDef {
    pub(crate) id: String,
    pub(crate) kind: NodeKindDef,
    pub(crate) range: [u64; 2],

    #[serde(default)]
    pub(crate) transform: TransformDef,
    #[serde(default = "default_opacity")]
    pub(crate) opacity: Anim<f64>,

    #[serde(default)]
    pub(crate) effects: Vec<EffectInstanceDef>,
    #[serde(default)]
    pub(crate) mask: Option<MaskDef>,
    #[serde(default)]
    pub(crate) transition_in: Option<TransitionSpecDef>,
    #[serde(default)]
    pub(crate) transition_out: Option<TransitionSpecDef>,
}

fn default_opacity() -> Anim<f64> {
    Anim::Constant(1.0)
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
    Switch { active: Anim<u64> },
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
        color: Option<VarDef>,
    },
    Video {
        source: String,
        #[serde(default)]
        trim_start_sec: f64,
        #[serde(default)]
        trim_end_sec: Option<f64>,
        #[serde(default = "default_playback_rate")]
        playback_rate: f64,
    },
    Audio {
        source: String,
        #[serde(default)]
        trim_start_sec: f64,
        #[serde(default)]
        trim_end_sec: Option<f64>,
        #[serde(default = "default_playback_rate")]
        playback_rate: f64,
    },
    Null,
}

fn default_playback_rate() -> f64 {
    1.0
}
