use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Keyframes<T> {
    pub(crate) keys: Vec<Keyframe<T>>,
    #[serde(default)]
    pub(crate) mode: InterpMode,
}

impl<'de, T> Deserialize<'de> for Keyframes<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr<T> {
            List(Vec<Keyframe<T>>),
            Obj {
                keys: Option<Vec<Keyframe<T>>>,
                mode: Option<InterpMode>,
            },
        }

        match Repr::deserialize(deserializer)? {
            Repr::List(keys) => Ok(Self {
                keys,
                mode: InterpMode::default(),
            }),
            Repr::Obj { keys, mode } => Ok(Self {
                keys: keys.unwrap_or_default(),
                mode: mode.unwrap_or_default(),
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Keyframe<T> {
    pub(crate) frame: u64,
    pub(crate) value: T,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InterpMode {
    #[default]
    Linear,
    Hold,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum Anim<T> {
    /// JSON shorthand: a bare value is a constant animation.
    Constant(T),
    /// Full form: tagged object.
    Tagged(AnimTagged<T>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnimTagged<T> {
    Keyframes(Keyframes<T>),
    /// Raw expression string (prefixed with `=`). Compiled during normalize.
    Expr(String),
}
