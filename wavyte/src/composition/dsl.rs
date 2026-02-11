use std::collections::BTreeMap;

use crate::{
    animation::anim::Anim,
    composition::model::{
        Asset, AudioAsset, BlendMode, Clip, ClipProps, Composition, EffectInstance, Track,
        TransitionSpec, VideoAsset,
    },
    foundation::core::{Canvas, FrameIndex, FrameRange, Transform2D},
    foundation::error::{WavyteError, WavyteResult},
};

/// Builder for [`Composition`](crate::Composition).
pub struct CompositionBuilder {
    fps: crate::foundation::core::Fps,
    canvas: Canvas,
    duration: FrameIndex,
    seed: u64,
    assets: BTreeMap<String, Asset>,
    tracks: Vec<Track>,
}

impl CompositionBuilder {
    /// Create a builder for a new composition.
    pub fn new(fps: crate::foundation::core::Fps, canvas: Canvas, duration: FrameIndex) -> Self {
        Self {
            fps,
            canvas,
            duration,
            seed: 0,
            assets: BTreeMap::new(),
            tracks: Vec::new(),
        }
    }

    /// Set global deterministic seed.
    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Insert an asset under unique key.
    pub fn asset(mut self, key: impl Into<String>, asset: Asset) -> WavyteResult<Self> {
        let key = key.into();
        if self.assets.contains_key(&key) {
            return Err(WavyteError::validation(format!(
                "duplicate asset key '{key}'"
            )));
        }
        self.assets.insert(key, asset);
        Ok(self)
    }

    /// Append a track to the composition.
    pub fn track(mut self, track: Track) -> Self {
        self.tracks.push(track);
        self
    }

    /// Convenience helper to add a [`VideoAsset`](crate::VideoAsset).
    pub fn video_asset(
        self,
        key: impl Into<String>,
        source: impl Into<String>,
    ) -> WavyteResult<Self> {
        self.asset(key, Asset::Video(video_asset(source)))
    }

    /// Convenience helper to add an [`AudioAsset`](crate::AudioAsset).
    pub fn audio_asset(
        self,
        key: impl Into<String>,
        source: impl Into<String>,
    ) -> WavyteResult<Self> {
        self.asset(key, Asset::Audio(audio_asset(source)))
    }

    /// Build and validate final [`Composition`](crate::Composition).
    pub fn build(self) -> WavyteResult<Composition> {
        let comp = Composition {
            fps: self.fps,
            canvas: self.canvas,
            duration: self.duration,
            assets: self.assets,
            tracks: self.tracks,
            seed: self.seed,
        };
        comp.validate()?;
        Ok(comp)
    }
}

/// Create video asset configuration with default trims/playback controls.
pub fn video_asset(source: impl Into<String>) -> VideoAsset {
    VideoAsset {
        source: source.into(),
        trim_start_sec: 0.0,
        trim_end_sec: None,
        playback_rate: 1.0,
        volume: 1.0,
        fade_in_sec: 0.0,
        fade_out_sec: 0.0,
        muted: false,
    }
}

/// Create audio asset configuration with default trims/playback controls.
pub fn audio_asset(source: impl Into<String>) -> AudioAsset {
    AudioAsset {
        source: source.into(),
        trim_start_sec: 0.0,
        trim_end_sec: None,
        playback_rate: 1.0,
        volume: 1.0,
        fade_in_sec: 0.0,
        fade_out_sec: 0.0,
        muted: false,
    }
}

/// Builder for [`Track`](crate::Track) values.
pub struct TrackBuilder {
    name: String,
    z_base: i32,
    layout_mode: crate::LayoutMode,
    layout_gap_px: f64,
    layout_padding: crate::Edges,
    layout_align_x: crate::LayoutAlignX,
    layout_align_y: crate::LayoutAlignY,
    layout_grid_columns: u32,
    clips: Vec<Clip>,
}

impl TrackBuilder {
    /// Create a track builder with required `name`.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            z_base: 0,
            layout_mode: crate::LayoutMode::Absolute,
            layout_gap_px: 0.0,
            layout_padding: crate::Edges::default(),
            layout_align_x: crate::LayoutAlignX::Start,
            layout_align_y: crate::LayoutAlignY::Start,
            layout_grid_columns: 2,
            clips: Vec::new(),
        }
    }

    /// Set base z-order for all clips in track.
    pub fn z_base(mut self, z: i32) -> Self {
        self.z_base = z;
        self
    }

    /// Append clip to the track.
    pub fn clip(mut self, clip: Clip) -> Self {
        self.clips.push(clip);
        self
    }

    /// Set track layout mode.
    pub fn layout_mode(mut self, mode: crate::LayoutMode) -> Self {
        self.layout_mode = mode;
        self
    }

    /// Set inter-item gap in pixels for stack/grid layouts.
    pub fn layout_gap_px(mut self, gap: f64) -> Self {
        self.layout_gap_px = gap;
        self
    }

    /// Set track layout padding.
    pub fn layout_padding(mut self, padding: crate::Edges) -> Self {
        self.layout_padding = padding;
        self
    }

    /// Set horizontal and vertical alignment for layout placement.
    pub fn layout_align(mut self, x: crate::LayoutAlignX, y: crate::LayoutAlignY) -> Self {
        self.layout_align_x = x;
        self.layout_align_y = y;
        self
    }

    /// Set number of columns for grid layout.
    pub fn layout_grid_columns(mut self, columns: u32) -> Self {
        self.layout_grid_columns = columns;
        self
    }

    /// Build validated [`Track`](crate::Track).
    pub fn build(self) -> WavyteResult<Track> {
        if self.name.trim().is_empty() {
            return Err(WavyteError::validation("track name must be non-empty"));
        }
        Ok(Track {
            name: self.name,
            z_base: self.z_base,
            layout_mode: self.layout_mode,
            layout_gap_px: self.layout_gap_px,
            layout_padding: self.layout_padding,
            layout_align_x: self.layout_align_x,
            layout_align_y: self.layout_align_y,
            layout_grid_columns: self.layout_grid_columns,
            clips: self.clips,
        })
    }
}

/// Builder for [`Clip`](crate::Clip) values.
pub struct ClipBuilder {
    id: String,
    asset_key: String,
    range: FrameRange,
    z_offset: i32,
    opacity: Anim<f64>,
    transform: Anim<Transform2D>,
    blend: BlendMode,
    effects: Vec<EffectInstance>,
    transition_in: Option<TransitionSpec>,
    transition_out: Option<TransitionSpec>,
}

impl ClipBuilder {
    /// Create clip builder with required identifiers and frame range.
    pub fn new(id: impl Into<String>, asset_key: impl Into<String>, range: FrameRange) -> Self {
        Self {
            id: id.into(),
            asset_key: asset_key.into(),
            range,
            z_offset: 0,
            opacity: Anim::constant(1.0),
            transform: Anim::constant(Transform2D::default()),
            blend: BlendMode::Normal,
            effects: Vec::new(),
            transition_in: None,
            transition_out: None,
        }
    }

    /// Set per-clip z-offset.
    pub fn z_offset(mut self, z: i32) -> Self {
        self.z_offset = z;
        self
    }

    /// Set animated opacity.
    pub fn opacity(mut self, a: Anim<f64>) -> Self {
        self.opacity = a;
        self
    }

    /// Set animated transform.
    pub fn transform(mut self, t: Anim<Transform2D>) -> Self {
        self.transform = t;
        self
    }

    /// Append effect instance.
    pub fn effect(mut self, fx: EffectInstance) -> Self {
        self.effects.push(fx);
        self
    }

    /// Set transition-in specification.
    pub fn transition_in(mut self, tr: TransitionSpec) -> Self {
        self.transition_in = Some(tr);
        self
    }

    /// Set transition-out specification.
    pub fn transition_out(mut self, tr: TransitionSpec) -> Self {
        self.transition_out = Some(tr);
        self
    }

    /// Build validated [`Clip`](crate::Clip).
    pub fn build(self) -> WavyteResult<Clip> {
        if self.id.trim().is_empty() {
            return Err(WavyteError::validation("clip id must be non-empty"));
        }
        if self.asset_key.trim().is_empty() {
            return Err(WavyteError::validation("clip asset key must be non-empty"));
        }
        self.opacity.validate()?;
        self.transform.validate()?;

        Ok(Clip {
            id: self.id,
            asset: self.asset_key,
            range: self.range,
            props: ClipProps {
                transform: self.transform,
                opacity: self.opacity,
                blend: self.blend,
            },
            z_offset: self.z_offset,
            effects: self.effects,
            transition_in: self.transition_in,
            transition_out: self.transition_out,
        })
    }
}

#[cfg(test)]
#[path = "../../tests/unit/composition/dsl.rs"]
mod tests;
