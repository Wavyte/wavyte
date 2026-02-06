use std::collections::BTreeMap;

use crate::{
    anim::Anim,
    core::{Canvas, FrameIndex, FrameRange, Transform2D},
    error::{WavyteError, WavyteResult},
    model::{
        Asset, AudioAsset, BlendMode, Clip, ClipProps, Composition, EffectInstance, Track,
        TransitionSpec, VideoAsset,
    },
};

pub struct CompositionBuilder {
    fps: crate::core::Fps,
    canvas: Canvas,
    duration: FrameIndex,
    seed: u64,
    assets: BTreeMap<String, Asset>,
    tracks: Vec<Track>,
}

impl CompositionBuilder {
    pub fn new(fps: crate::core::Fps, canvas: Canvas, duration: FrameIndex) -> Self {
        Self {
            fps,
            canvas,
            duration,
            seed: 0,
            assets: BTreeMap::new(),
            tracks: Vec::new(),
        }
    }

    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

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

    pub fn track(mut self, track: Track) -> Self {
        self.tracks.push(track);
        self
    }

    pub fn video_asset(
        self,
        key: impl Into<String>,
        source: impl Into<String>,
    ) -> WavyteResult<Self> {
        self.asset(key, Asset::Video(video_asset(source)))
    }

    pub fn audio_asset(
        self,
        key: impl Into<String>,
        source: impl Into<String>,
    ) -> WavyteResult<Self> {
        self.asset(key, Asset::Audio(audio_asset(source)))
    }

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

    pub fn z_base(mut self, z: i32) -> Self {
        self.z_base = z;
        self
    }

    pub fn clip(mut self, clip: Clip) -> Self {
        self.clips.push(clip);
        self
    }

    pub fn layout_mode(mut self, mode: crate::LayoutMode) -> Self {
        self.layout_mode = mode;
        self
    }

    pub fn layout_gap_px(mut self, gap: f64) -> Self {
        self.layout_gap_px = gap;
        self
    }

    pub fn layout_padding(mut self, padding: crate::Edges) -> Self {
        self.layout_padding = padding;
        self
    }

    pub fn layout_align(mut self, x: crate::LayoutAlignX, y: crate::LayoutAlignY) -> Self {
        self.layout_align_x = x;
        self.layout_align_y = y;
        self
    }

    pub fn layout_grid_columns(mut self, columns: u32) -> Self {
        self.layout_grid_columns = columns;
        self
    }

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

    pub fn z_offset(mut self, z: i32) -> Self {
        self.z_offset = z;
        self
    }

    pub fn opacity(mut self, a: Anim<f64>) -> Self {
        self.opacity = a;
        self
    }

    pub fn transform(mut self, t: Anim<Transform2D>) -> Self {
        self.transform = t;
        self
    }

    pub fn effect(mut self, fx: EffectInstance) -> Self {
        self.effects.push(fx);
        self
    }

    pub fn transition_in(mut self, tr: TransitionSpec) -> Self {
        self.transition_in = Some(tr);
        self
    }

    pub fn transition_out(mut self, tr: TransitionSpec) -> Self {
        self.transition_out = Some(tr);
        self
    }

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
mod tests {
    use super::*;
    use crate::{
        anim_ease::Ease,
        core::{Fps, Vec2},
        model::{Asset, TextAsset},
    };

    #[test]
    fn builders_create_expected_structure() {
        let clip = ClipBuilder::new(
            "c0",
            "t0",
            FrameRange::new(crate::core::FrameIndex(0), crate::core::FrameIndex(30)).unwrap(),
        )
        .opacity(Anim::constant(0.5))
        .transform(Anim::constant(Transform2D {
            translate: Vec2::new(1.0, 2.0),
            ..Transform2D::default()
        }))
        .transition_in(TransitionSpec {
            kind: "crossfade".to_string(),
            duration_frames: 10,
            ease: Ease::Linear,
            params: serde_json::Value::Null,
        })
        .build()
        .unwrap();

        let track = TrackBuilder::new("main").clip(clip).build().unwrap();

        let comp = CompositionBuilder::new(
            Fps::new(30, 1).unwrap(),
            Canvas {
                width: 640,
                height: 360,
            },
            FrameIndex(30),
        )
        .asset(
            "t0",
            Asset::Text(TextAsset {
                text: "hello".to_string(),
                font_source: "assets/PlayfairDisplay.ttf".to_string(),
                size_px: 48.0,
                max_width_px: None,
                color_rgba8: [255, 255, 255, 255],
            }),
        )
        .unwrap()
        .track(track)
        .build()
        .unwrap();

        assert_eq!(comp.assets.len(), 1);
        assert_eq!(comp.tracks.len(), 1);
    }

    #[test]
    fn duplicate_asset_key_is_rejected() {
        let builder = CompositionBuilder::new(
            Fps::new(30, 1).unwrap(),
            Canvas {
                width: 640,
                height: 360,
            },
            FrameIndex(1),
        )
        .asset(
            "t0",
            Asset::Text(TextAsset {
                text: "a".into(),
                font_source: "assets/PlayfairDisplay.ttf".to_string(),
                size_px: 48.0,
                max_width_px: None,
                color_rgba8: [255, 255, 255, 255],
            }),
        )
        .unwrap();
        assert!(
            builder
                .asset(
                    "t0",
                    Asset::Text(TextAsset {
                        text: "b".into(),
                        font_source: "assets/PlayfairDisplay.ttf".to_string(),
                        size_px: 48.0,
                        max_width_px: None,
                        color_rgba8: [255, 255, 255, 255],
                    }),
                )
                .is_err()
        );
    }
}
