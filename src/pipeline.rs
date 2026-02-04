use crate::{
    assets::AssetCache,
    compile::compile_frame,
    core::{FrameIndex, FrameRange},
    error::{WavyteError, WavyteResult},
    eval::Evaluator,
    model::Composition,
    render::{FrameRGBA, RenderBackend},
    render_passes::execute_plan,
};

pub fn render_frame(
    comp: &Composition,
    frame: FrameIndex,
    backend: &mut dyn RenderBackend,
    assets: &mut dyn AssetCache,
) -> WavyteResult<FrameRGBA> {
    let eval = Evaluator::eval_frame(comp, frame)?;
    let plan = compile_frame(comp, &eval, assets)?;
    execute_plan(backend, &plan, assets)
}

pub fn render_frames(
    comp: &Composition,
    range: FrameRange,
    backend: &mut dyn RenderBackend,
    assets: &mut dyn AssetCache,
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

#[cfg(feature = "ffmpeg")]
#[derive(Clone, Debug)]
pub struct RenderToMp4Opts {
    pub range: FrameRange,
    pub bg_rgba: [u8; 4],
    pub overwrite: bool,
}

#[cfg(feature = "ffmpeg")]
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

#[cfg(feature = "ffmpeg")]
pub fn render_to_mp4(
    comp: &Composition,
    out_path: impl Into<std::path::PathBuf>,
    opts: RenderToMp4Opts,
    backend: &mut dyn RenderBackend,
    assets: &mut dyn AssetCache,
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
