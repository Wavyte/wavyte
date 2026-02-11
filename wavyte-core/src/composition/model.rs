use std::collections::BTreeMap;

use crate::{
    animation::anim::Anim,
    animation::ease::Ease,
    foundation::core::{Canvas, Fps, FrameIndex, FrameRange, Transform2D},
    foundation::error::{WavyteError, WavyteResult},
};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
/// A complete timeline composition.
///
/// A composition is a pure data model that can be:
/// - built programmatically (see [`crate::CompositionBuilder`])
/// - serialized/deserialized via Serde (JSON)
///
/// Rendering a composition is performed by the pipeline:
/// [`crate::render_frame`] / [`crate::render_to_mp4`].
pub struct Composition {
    pub fps: Fps,
    pub canvas: Canvas,
    pub duration: FrameIndex,            // total frames
    pub assets: BTreeMap<String, Asset>, // stable keys
    pub tracks: Vec<Track>,
    pub seed: u64, // global determinism seed
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
/// A track contains an ordered set of clips with a base Z offset.
pub struct Track {
    pub name: String,
    pub z_base: i32,
    #[serde(default)]
    pub layout_mode: LayoutMode,
    #[serde(default)]
    pub layout_gap_px: f64,
    #[serde(default)]
    pub layout_padding: Edges,
    #[serde(default)]
    pub layout_align_x: LayoutAlignX,
    #[serde(default)]
    pub layout_align_y: LayoutAlignY,
    #[serde(default = "default_layout_grid_columns")]
    pub layout_grid_columns: u32,
    pub clips: Vec<Clip>,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum LayoutMode {
    #[default]
    Absolute,
    HStack,
    VStack,
    Grid,
    Center,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Edges {
    #[serde(default)]
    pub left: f64,
    #[serde(default)]
    pub right: f64,
    #[serde(default)]
    pub top: f64,
    #[serde(default)]
    pub bottom: f64,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum LayoutAlignX {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum LayoutAlignY {
    #[default]
    Start,
    Center,
    End,
}

fn default_layout_grid_columns() -> u32 {
    2
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
/// A clip places an asset on the timeline and specifies how it is rendered.
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
/// Per-clip render properties (animated).
pub struct ClipProps {
    pub transform: Anim<Transform2D>,
    pub opacity: Anim<f64>, // 0..1 clamped in eval
    pub blend: BlendMode,
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
/// Blend mode used when compositing a clip.
pub enum BlendMode {
    /// Standard “source over destination” (premultiplied alpha).
    Normal,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
/// An asset referenced by clips.
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
    pub font_source: String,
    pub size_px: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_width_px: Option<f32>,
    #[serde(default = "default_text_color_rgba8")]
    pub color_rgba8: [u8; 4],
}

fn default_text_color_rgba8() -> [u8; 4] {
    [255, 255, 255, 255]
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
    #[serde(default)]
    pub trim_start_sec: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trim_end_sec: Option<f64>,
    #[serde(default = "default_playback_rate")]
    pub playback_rate: f64,
    #[serde(default = "default_volume")]
    pub volume: f64,
    #[serde(default)]
    pub fade_in_sec: f64,
    #[serde(default)]
    pub fade_out_sec: f64,
    #[serde(default)]
    pub muted: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AudioAsset {
    pub source: String,
    #[serde(default)]
    pub trim_start_sec: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trim_end_sec: Option<f64>,
    #[serde(default = "default_playback_rate")]
    pub playback_rate: f64,
    #[serde(default = "default_volume")]
    pub volume: f64,
    #[serde(default)]
    pub fade_in_sec: f64,
    #[serde(default)]
    pub fade_out_sec: f64,
    #[serde(default)]
    pub muted: bool,
}

fn default_playback_rate() -> f64 {
    1.0
}

fn default_volume() -> f64 {
    1.0
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
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub params: serde_json::Value,
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
            if !track.layout_gap_px.is_finite() || track.layout_gap_px < 0.0 {
                return Err(WavyteError::validation(
                    "track layout_gap_px must be finite and >= 0",
                ));
            }
            for (name, value) in [
                ("left", track.layout_padding.left),
                ("right", track.layout_padding.right),
                ("top", track.layout_padding.top),
                ("bottom", track.layout_padding.bottom),
            ] {
                if !value.is_finite() || value < 0.0 {
                    return Err(WavyteError::validation(format!(
                        "track layout_padding.{name} must be finite and >= 0",
                    )));
                }
            }
            if track.layout_mode == LayoutMode::Grid && track.layout_grid_columns == 0 {
                return Err(WavyteError::validation(
                    "track layout_grid_columns must be > 0 for Grid layout",
                ));
            }

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

        for (key, asset) in &self.assets {
            if key.trim().is_empty() {
                return Err(WavyteError::validation("asset key must be non-empty"));
            }
            match asset {
                Asset::Text(a) => {
                    if a.text.trim().is_empty() {
                        return Err(WavyteError::validation("text asset text must be non-empty"));
                    }
                    validate_rel_source(&a.font_source, "text asset font_source")?;
                    if !a.size_px.is_finite() || a.size_px <= 0.0 {
                        return Err(WavyteError::validation(
                            "text asset size_px must be finite and > 0",
                        ));
                    }
                    if let Some(w) = a.max_width_px
                        && (!w.is_finite() || w <= 0.0)
                    {
                        return Err(WavyteError::validation(
                            "text asset max_width_px must be finite and > 0 when set",
                        ));
                    }
                }
                Asset::Svg(a) => validate_rel_source(&a.source, "svg asset source")?,
                Asset::Image(a) => validate_rel_source(&a.source, "image asset source")?,
                Asset::Video(a) => {
                    validate_rel_source(&a.source, "video asset source")?;
                    validate_media_controls(
                        a.trim_start_sec,
                        a.trim_end_sec,
                        a.playback_rate,
                        a.volume,
                        a.fade_in_sec,
                        a.fade_out_sec,
                        "video asset",
                    )?;
                }
                Asset::Audio(a) => {
                    validate_rel_source(&a.source, "audio asset source")?;
                    validate_media_controls(
                        a.trim_start_sec,
                        a.trim_end_sec,
                        a.playback_rate,
                        a.volume,
                        a.fade_in_sec,
                        a.fade_out_sec,
                        "audio asset",
                    )?;
                }
                Asset::Path(a) => {
                    if a.svg_path_d.trim().is_empty() {
                        return Err(WavyteError::validation(
                            "path asset svg_path_d must be non-empty",
                        ));
                    }
                }
            }
        }

        Ok(())
    }
}

fn validate_rel_source(source: &str, field: &str) -> WavyteResult<()> {
    if source.trim().is_empty() {
        return Err(WavyteError::validation(format!(
            "{field} must be non-empty"
        )));
    }
    let s = source.replace('\\', "/");
    if s.starts_with('/') {
        return Err(WavyteError::validation(format!(
            "{field} must be a relative path"
        )));
    }
    for part in s.split('/') {
        if part == ".." {
            return Err(WavyteError::validation(format!(
                "{field} must not contain '..'"
            )));
        }
    }
    Ok(())
}

fn validate_media_controls(
    trim_start_sec: f64,
    trim_end_sec: Option<f64>,
    playback_rate: f64,
    volume: f64,
    fade_in_sec: f64,
    fade_out_sec: f64,
    kind: &str,
) -> WavyteResult<()> {
    if !trim_start_sec.is_finite() || trim_start_sec < 0.0 {
        return Err(WavyteError::validation(format!(
            "{kind} trim_start_sec must be finite and >= 0",
        )));
    }
    if let Some(end) = trim_end_sec
        && (!end.is_finite() || end <= trim_start_sec)
    {
        return Err(WavyteError::validation(format!(
            "{kind} trim_end_sec must be finite and > trim_start_sec",
        )));
    }
    if !playback_rate.is_finite() || playback_rate <= 0.0 {
        return Err(WavyteError::validation(format!(
            "{kind} playback_rate must be finite and > 0",
        )));
    }
    if !volume.is_finite() || volume < 0.0 {
        return Err(WavyteError::validation(format!(
            "{kind} volume must be finite and >= 0",
        )));
    }
    if !fade_in_sec.is_finite() || fade_in_sec < 0.0 {
        return Err(WavyteError::validation(format!(
            "{kind} fade_in_sec must be finite and >= 0",
        )));
    }
    if !fade_out_sec.is_finite() || fade_out_sec < 0.0 {
        return Err(WavyteError::validation(format!(
            "{kind} fade_out_sec must be finite and >= 0",
        )));
    }
    Ok(())
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
        if !(self.params.is_null() || self.params.is_object()) {
            return Err(WavyteError::validation(
                "transition params must be an object when set",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../tests/unit/composition/model.rs"]
mod tests;
