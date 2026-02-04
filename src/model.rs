use std::collections::BTreeMap;

use crate::{
    anim::Anim,
    anim_ease::Ease,
    core::{Canvas, Fps, FrameIndex, FrameRange, Transform2D},
    error::{WavyteError, WavyteResult},
};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Composition {
    pub fps: Fps,
    pub canvas: Canvas,
    pub duration: FrameIndex,            // total frames
    pub assets: BTreeMap<String, Asset>, // stable keys
    pub tracks: Vec<Track>,
    pub seed: u64, // global determinism seed
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Track {
    pub name: String,
    pub z_base: i32,
    pub clips: Vec<Clip>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Clip {
    pub id: String,
    pub asset: String,     // key into Composition.assets
    pub range: FrameRange, // timeline placement [start,end)
    pub props: ClipProps,
    pub z_offset: i32,
    pub effects: Vec<EffectInstance>,
    pub transition_in: Option<TransitionSpec>,
    pub transition_out: Option<TransitionSpec>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ClipProps {
    pub transform: Anim<Transform2D>,
    pub opacity: Anim<f64>, // 0..1 clamped in eval
    pub blend: BlendMode,
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub enum BlendMode {
    Normal,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Asset {
    Text(TextAsset),
    Svg(SvgAsset),
    Path(PathAsset),
    Image(ImageAsset),
    Video(VideoAsset),
    Audio(AudioAsset),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TextAsset {
    pub text: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SvgAsset {
    pub source: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PathAsset {
    pub svg_path_d: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ImageAsset {
    pub source: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct VideoAsset {
    pub source: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AudioAsset {
    pub source: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct EffectInstance {
    pub kind: String,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub params: serde_json::Value,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TransitionSpec {
    pub kind: String,
    pub duration_frames: u64,
    pub ease: Ease,
}

impl Composition {
    pub fn validate(&self) -> WavyteResult<()> {
        if self.fps.num == 0 || self.fps.den == 0 {
            return Err(WavyteError::validation("fps must have num>0 and den>0"));
        }
        if self.canvas.width == 0 || self.canvas.height == 0 {
            return Err(WavyteError::validation("canvas width/height must be > 0"));
        }
        if self.duration.0 == 0 {
            return Err(WavyteError::validation("duration must be > 0 frames"));
        }

        for track in &self.tracks {
            for clip in &track.clips {
                if !self.assets.contains_key(&clip.asset) {
                    return Err(WavyteError::validation(format!(
                        "clip '{}' references missing asset key '{}'",
                        clip.id, clip.asset
                    )));
                }
                if clip.range.start.0 > clip.range.end.0 {
                    return Err(WavyteError::validation(format!(
                        "clip '{}' has invalid range (start > end)",
                        clip.id
                    )));
                }
                if clip.range.end.0 > self.duration.0 {
                    return Err(WavyteError::validation(format!(
                        "clip '{}' range exceeds composition duration",
                        clip.id
                    )));
                }

                clip.props.opacity.validate()?;
                clip.props.transform.validate()?;

                if let Some(tr) = &clip.transition_in {
                    tr.validate()?;
                }
                if let Some(tr) = &clip.transition_out {
                    tr.validate()?;
                }
            }
        }

        Ok(())
    }
}

impl TransitionSpec {
    pub fn validate(&self) -> WavyteResult<()> {
        if self.kind.trim().is_empty() {
            return Err(WavyteError::validation("transition kind must be non-empty"));
        }
        if self.duration_frames == 0 {
            return Err(WavyteError::validation(
                "transition duration_frames must be > 0",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Vec2;

    fn basic_comp() -> Composition {
        let mut assets = BTreeMap::new();
        assets.insert(
            "t0".to_string(),
            Asset::Text(TextAsset {
                text: "hello".to_string(),
            }),
        );
        Composition {
            fps: Fps::new(30, 1).unwrap(),
            canvas: Canvas {
                width: 1920,
                height: 1080,
            },
            duration: FrameIndex(60),
            assets,
            tracks: vec![Track {
                name: "main".to_string(),
                z_base: 0,
                clips: vec![Clip {
                    id: "c0".to_string(),
                    asset: "t0".to_string(),
                    range: FrameRange::new(FrameIndex(0), FrameIndex(60)).unwrap(),
                    props: ClipProps {
                        transform: Anim::constant(Transform2D {
                            translate: Vec2::new(10.0, 20.0),
                            ..Transform2D::default()
                        }),
                        opacity: Anim::constant(1.0),
                        blend: BlendMode::Normal,
                    },
                    z_offset: 0,
                    effects: vec![EffectInstance {
                        kind: "noop".to_string(),
                        params: serde_json::Value::Null,
                    }],
                    transition_in: Some(TransitionSpec {
                        kind: "crossfade".to_string(),
                        duration_frames: 10,
                        ease: Ease::Linear,
                    }),
                    transition_out: None,
                }],
            }],
            seed: 123,
        }
    }

    #[test]
    fn json_roundtrip() {
        let comp = basic_comp();
        let s = serde_json::to_string_pretty(&comp).unwrap();
        let de: Composition = serde_json::from_str(&s).unwrap();
        assert_eq!(de.canvas.width, 1920);
        assert_eq!(de.assets.len(), 1);
    }

    #[test]
    fn validate_rejects_missing_asset() {
        let mut comp = basic_comp();
        comp.tracks[0].clips[0].asset = "missing".to_string();
        assert!(comp.validate().is_err());
    }

    #[test]
    fn validate_rejects_out_of_bounds_range() {
        let mut comp = basic_comp();
        comp.tracks[0].clips[0].range = FrameRange {
            start: FrameIndex(0),
            end: FrameIndex(999),
        };
        assert!(comp.validate().is_err());
    }

    #[test]
    fn validate_rejects_bad_fps() {
        let mut comp = basic_comp();
        comp.fps = Fps { num: 30, den: 0 };
        assert!(comp.validate().is_err());
    }
}
