use crate::{
    asset_store::PreparedAssetStore,
    compile::compile_frame,
    core::{FrameIndex, FrameRange},
    error::{WavyteError, WavyteResult},
    eval::Evaluator,
    model::Composition,
    render::{FrameRGBA, RenderBackend},
    render_passes::execute_plan,
};

/// Evaluate + compile + render a single frame.
///
/// This is the primary “one-shot” API for producing pixels from a [`Composition`].
///
/// Pipeline:
/// 1. [`Evaluator::eval_frame`](crate::Evaluator::eval_frame)
/// 2. [`compile_frame`](crate::compile_frame)
/// 3. [`RenderBackend::render_plan`](crate::RenderBackend::render_plan)
///
/// Returns a [`FrameRGBA`] containing **premultiplied** RGBA8 pixels.
pub fn render_frame(
    comp: &Composition,
    frame: FrameIndex,
    backend: &mut dyn RenderBackend,
    assets: &PreparedAssetStore,
) -> WavyteResult<FrameRGBA> {
    let eval = Evaluator::eval_frame(comp, frame)?;
    let plan = compile_frame(comp, &eval, assets)?;
    execute_plan(backend, &plan, assets)
}

/// Render a range of frames (inclusive start, exclusive end).
///
/// This is a convenience wrapper that repeatedly calls [`render_frame`].
pub fn render_frames(
    comp: &Composition,
    range: FrameRange,
    backend: &mut dyn RenderBackend,
    assets: &PreparedAssetStore,
) -> WavyteResult<Vec<FrameRGBA>> {
    if range.is_empty() {
        return Err(WavyteError::validation("render range must be non-empty"));
    }

    let len = range.len_frames();
    let mut out = Vec::with_capacity(len.min(4096) as usize);
    for f in range.start.0..range.end.0 {
        out.push(render_frame(comp, FrameIndex(f), backend, assets)?);
    }
    Ok(out)
}

/// Options for [`render_to_mp4`].
///
/// `bg_rgba` is used when flattening alpha for the encoder.
#[derive(Clone, Debug)]
pub struct RenderToMp4Opts {
    /// Frame range to render (start inclusive, end exclusive).
    pub range: FrameRange,
    /// Background color to flatten alpha over (RGBA8, straight alpha).
    pub bg_rgba: [u8; 4],
    /// Whether to overwrite `out_path` if it already exists.
    pub overwrite: bool,
}

impl Default for RenderToMp4Opts {
    fn default() -> Self {
        Self {
            range: FrameRange {
                start: FrameIndex(0),
                end: FrameIndex(1),
            },
            bg_rgba: [0, 0, 0, 255],
            overwrite: true,
        }
    }
}

/// Render a composition to an MP4 by invoking the system `ffmpeg` binary.
///
/// `ffmpeg` must be installed and on `PATH`. This function checks for it up front and returns an
/// error if it is not available.
///
/// Notes:
/// - v0.1.0 currently requires integer FPS (`comp.fps.den == 1`) for MP4 output.
/// - Frames are rendered as premultiplied RGBA8; the encoder can flatten alpha over `bg_rgba`.
pub fn render_to_mp4(
    comp: &Composition,
    out_path: impl Into<std::path::PathBuf>,
    opts: RenderToMp4Opts,
    backend: &mut dyn RenderBackend,
    assets: &PreparedAssetStore,
) -> WavyteResult<()> {
    if opts.range.end.0 > comp.duration.0 {
        return Err(WavyteError::validation(
            "render_to_mp4 range must be within composition duration",
        ));
    }
    if opts.range.is_empty() {
        return Err(WavyteError::validation(
            "render_to_mp4 range must be non-empty",
        ));
    }

    let fps = if comp.fps.den == 1 {
        comp.fps.num
    } else {
        return Err(WavyteError::validation(
            "render_to_mp4 currently requires integer fps (fps.den == 1)",
        ));
    };

    let out_path = out_path.into();
    if !crate::encode_ffmpeg::is_ffmpeg_on_path() {
        return Err(WavyteError::evaluation(
            "ffmpeg is required for MP4 rendering, but was not found on PATH",
        ));
    }

    let cfg = crate::encode_ffmpeg::EncodeConfig {
        width: comp.canvas.width,
        height: comp.canvas.height,
        fps,
        out_path,
        overwrite: opts.overwrite,
    };

    let mut enc = crate::encode_ffmpeg::FfmpegEncoder::new(cfg, opts.bg_rgba)?;
    for f in opts.range.start.0..opts.range.end.0 {
        let frame = render_frame(comp, FrameIndex(f), backend, assets)?;
        enc.encode_frame(&frame)?;
    }
    enc.finish()
}
