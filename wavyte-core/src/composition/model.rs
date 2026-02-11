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
    /// Timeline frame rate.
    pub fps: Fps,
    /// Output canvas dimensions.
    pub canvas: Canvas,
    /// Total composition duration in frames.
    pub duration: FrameIndex, // total frames
    /// Asset table keyed by stable user-facing asset keys.
    pub assets: BTreeMap<String, Asset>, // stable keys
    /// Ordered tracks in composition.
    pub tracks: Vec<Track>,
    /// Global deterministic seed used by procedural animation sources.
    pub seed: u64, // global determinism seed
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
/// A track contains an ordered set of clips with a base Z offset.
pub struct Track {
    /// Track name for authoring/debugging.
    pub name: String,
    /// Base z-order applied to all clips in this track.
    pub z_base: i32,
    /// Layout mode controlling auto-placement of clips.
    #[serde(default)]
    pub layout_mode: LayoutMode,
    /// Gap in pixels between items in stack/grid layouts.
    #[serde(default)]
    pub layout_gap_px: f64,
    /// Padding around layout container in pixels.
    #[serde(default)]
    pub layout_padding: Edges,
    /// Horizontal alignment inside available container/cell.
    #[serde(default)]
    pub layout_align_x: LayoutAlignX,
    /// Vertical alignment inside available container/cell.
    #[serde(default)]
    pub layout_align_y: LayoutAlignY,
    /// Column count used for grid layout.
    #[serde(default = "default_layout_grid_columns")]
    pub layout_grid_columns: u32,
    /// Clips contained in this track.
    pub clips: Vec<Clip>,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
/// Auto-layout mode for clips within a track.
pub enum LayoutMode {
    /// Do not auto-place clips; use clip transforms only.
    #[default]
    Absolute,
    /// Horizontal stack from left to right.
    HStack,
    /// Vertical stack from top to bottom.
    VStack,
    /// Uniform grid layout.
    Grid,
    /// Center each clip in available track box.
    Center,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
/// Padding edges in pixels.
pub struct Edges {
    /// Left padding.
    #[serde(default)]
    pub left: f64,
    /// Right padding.
    #[serde(default)]
    pub right: f64,
    /// Top padding.
    #[serde(default)]
    pub top: f64,
    /// Bottom padding.
    #[serde(default)]
    pub bottom: f64,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
/// Horizontal alignment options for layout.
pub enum LayoutAlignX {
    /// Align to start (left).
    #[default]
    Start,
    /// Align to center.
    Center,
    /// Align to end (right).
    End,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
/// Vertical alignment options for layout.
pub enum LayoutAlignY {
    /// Align to start (top).
    #[default]
    Start,
    /// Align to center.
    Center,
    /// Align to end (bottom).
    End,
}

fn default_layout_grid_columns() -> u32 {
    2
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
/// A clip places an asset on the timeline and specifies how it is rendered.
pub struct Clip {
    /// Clip identifier (stable within a composition).
    pub id: String,
    /// Asset key into [`Composition::assets`].
    pub asset: String, // key into Composition.assets
    /// Timeline placement range `[start, end)`.
    pub range: FrameRange, // timeline placement [start,end)
    /// Animated clip render properties.
    pub props: ClipProps,
    /// Per-clip z-order offset added on top of track base.
    pub z_offset: i32,
    /// Per-clip effect stack.
    pub effects: Vec<EffectInstance>,
    /// Optional transition-in specification.
    pub transition_in: Option<TransitionSpec>,
    /// Optional transition-out specification.
    pub transition_out: Option<TransitionSpec>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
/// Per-clip render properties (animated).
pub struct ClipProps {
    /// Animated transform.
    pub transform: Anim<Transform2D>,
    /// Animated opacity; clamped to `[0, 1]` at evaluation time.
    pub opacity: Anim<f64>, // 0..1 clamped in eval
    /// Blend mode.
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
    /// Text asset.
    Text(TextAsset),
    /// SVG file asset.
    Svg(SvgAsset),
    /// Inline SVG path asset.
    Path(PathAsset),
    /// Raster image asset.
    Image(ImageAsset),
    /// Video file asset.
    Video(VideoAsset),
    /// Audio file asset.
    Audio(AudioAsset),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
/// Text asset configuration.
pub struct TextAsset {
    /// UTF-8 text content.
    pub text: String,
    /// Relative path to font file.
    pub font_source: String,
    /// Font size in pixels.
    pub size_px: f32,
    /// Optional max line width in pixels (for wrapping).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_width_px: Option<f32>,
    /// Text color as straight-alpha RGBA8.
    #[serde(default = "default_text_color_rgba8")]
    pub color_rgba8: [u8; 4],
}

fn default_text_color_rgba8() -> [u8; 4] {
    [255, 255, 255, 255]
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
/// SVG asset configuration.
pub struct SvgAsset {
    /// Relative path to SVG file.
    pub source: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
/// Inline path asset configuration.
pub struct PathAsset {
    /// SVG path `d` attribute string.
    pub svg_path_d: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
/// Raster image asset configuration.
pub struct ImageAsset {
    /// Relative path to image file.
    pub source: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
/// Video asset configuration including trims and audio controls.
pub struct VideoAsset {
    /// Relative path to video file.
    pub source: String,
    /// Source trim start in seconds.
    #[serde(default)]
    pub trim_start_sec: f64,
    /// Optional source trim end in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trim_end_sec: Option<f64>,
    /// Source playback rate multiplier.
    #[serde(default = "default_playback_rate")]
    pub playback_rate: f64,
    /// Audio volume multiplier.
    #[serde(default = "default_volume")]
    pub volume: f64,
    /// Audio fade-in duration in seconds.
    #[serde(default)]
    pub fade_in_sec: f64,
    /// Audio fade-out duration in seconds.
    #[serde(default)]
    pub fade_out_sec: f64,
    /// Disable video-audio contribution when `true`.
    #[serde(default)]
    pub muted: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
/// Audio asset configuration including trims and fades.
pub struct AudioAsset {
    /// Relative path to audio file.
    pub source: String,
    /// Source trim start in seconds.
    #[serde(default)]
    pub trim_start_sec: f64,
    /// Optional source trim end in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trim_end_sec: Option<f64>,
    /// Source playback rate multiplier.
    #[serde(default = "default_playback_rate")]
    pub playback_rate: f64,
    /// Audio volume multiplier.
    #[serde(default = "default_volume")]
    pub volume: f64,
    /// Fade-in duration in seconds.
    #[serde(default)]
    pub fade_in_sec: f64,
    /// Fade-out duration in seconds.
    #[serde(default)]
    pub fade_out_sec: f64,
    /// Disable contribution when `true`.
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
/// Raw effect instance attached to a clip.
pub struct EffectInstance {
    /// Effect kind identifier.
    pub kind: String,
    /// Effect parameter object.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub params: serde_json::Value,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
/// Transition specification attached to clip edge.
pub struct TransitionSpec {
    /// Transition kind identifier.
    pub kind: String,
    /// Transition duration in frames.
    pub duration_frames: u64,
    /// Easing applied to transition progress.
    pub ease: Ease,
    /// Transition parameter object.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub params: serde_json::Value,
}

impl Composition {
    /// Validate composition invariants and asset/clip references.
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
    /// Validate transition payload invariants.
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
