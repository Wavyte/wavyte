use std::collections::HashMap;

use rayon::prelude::*;

use crate::{
    assets::store::PreparedAssetStore,
    compile::fingerprint::{FrameFingerprint, fingerprint_eval},
    compile::plan::{CompileCache, compile_frame_with_cache},
    composition::model::Composition,
    eval::evaluator::Evaluator,
    foundation::core::{FrameIndex, FrameRange},
    foundation::error::{WavyteError, WavyteResult},
    render::backend::{FrameRGBA, RenderBackend, RenderSettings},
    render::passes::execute_plan,
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
    comp.validate()?;
    let layout_offsets = crate::resolve_layout_offsets(comp, assets)?;
    let eval = Evaluator::eval_frame_with_layout_unchecked(comp, frame, &layout_offsets)?;
    let mut compile_cache = CompileCache::default();
    let plan = compile_frame_with_cache(comp, &eval, assets, &mut compile_cache)?;
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
    render_frames_with_stats(comp, range, backend, assets, &RenderThreading::default())
        .map(|(frames, _)| frames)
}

#[derive(Clone, Debug)]
/// Threading and chunking controls for multi-frame rendering.
pub struct RenderThreading {
    /// Enable parallel rendering when `true`.
    pub parallel: bool,
    /// Chunk size in frames for batched scheduling.
    pub chunk_size: usize,
    /// Optional explicit worker thread count.
    pub threads: Option<usize>,
    /// Enable static-frame fingerprint elision in parallel mode.
    pub static_frame_elision: bool,
}

impl Default for RenderThreading {
    fn default() -> Self {
        Self {
            parallel: false,
            chunk_size: 64,
            threads: None,
            static_frame_elision: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
/// Aggregated rendering counters.
pub struct RenderStats {
    /// Total requested frames.
    pub frames_total: u64,
    /// Frames that were actually rendered.
    pub frames_rendered: u64,
    /// Frames reused via static-frame elision.
    pub frames_elided: u64,
}

/// Render a frame range and return both frame data and rendering stats.
pub fn render_frames_with_stats(
    comp: &Composition,
    range: FrameRange,
    backend: &mut dyn RenderBackend,
    assets: &PreparedAssetStore,
    threading: &RenderThreading,
) -> WavyteResult<(Vec<FrameRGBA>, RenderStats)> {
    if range.is_empty() {
        return Err(WavyteError::validation("render range must be non-empty"));
    }
    comp.validate()?;

    let len = range.len_frames();
    let mut out = Vec::with_capacity(len.min(4096) as usize);
    let mut stats = RenderStats::default();
    let chunk_size = normalized_chunk_size(threading.chunk_size);
    let layout_offsets = crate::resolve_layout_offsets(comp, assets)?;
    let mut compile_cache = CompileCache::default();

    if !threading.parallel {
        for f in range.start.0..range.end.0 {
            let eval =
                Evaluator::eval_frame_with_layout_unchecked(comp, FrameIndex(f), &layout_offsets)?;
            let plan = compile_frame_with_cache(comp, &eval, assets, &mut compile_cache)?;
            out.push(execute_plan(backend, &plan, assets)?);
            stats.frames_total += 1;
            stats.frames_rendered += 1;
        }
        return Ok((out, stats));
    }

    let worker_settings = backend.worker_render_settings().ok_or_else(|| {
        WavyteError::evaluation(
            "parallel render requires backend worker settings support (CpuBackend)",
        )
    })?;
    let pool = build_thread_pool(threading.threads)?;

    let mut chunk_start = range.start.0;
    while chunk_start < range.end.0 {
        let chunk_end = (chunk_start + chunk_size).min(range.end.0);
        let chunk = FrameRange::new(FrameIndex(chunk_start), FrameIndex(chunk_end))
            .map_err(|e| WavyteError::evaluation(format!("invalid chunk range: {e}")))?;
        let (mut frames, chunk_stats) = render_chunk_parallel_cpu(
            comp,
            chunk,
            assets,
            &worker_settings,
            threading,
            &pool,
            &layout_offsets,
        )?;
        out.append(&mut frames);
        stats.frames_total += chunk_stats.frames_total;
        stats.frames_rendered += chunk_stats.frames_rendered;
        stats.frames_elided += chunk_stats.frames_elided;
        chunk_start = chunk_end;
    }

    Ok((out, stats))
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
    /// Render threading/chunking configuration.
    pub threading: RenderThreading,
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
            threading: RenderThreading::default(),
        }
    }
}

/// Render a composition to an MP4 by invoking the system `ffmpeg` binary.
///
/// `ffmpeg` must be installed and on `PATH`. This function checks for it up front and returns an
/// error if it is not available.
///
/// Notes:
/// - v0.2.1 currently requires integer FPS (`comp.fps.den == 1`) for MP4 output.
/// - Frames are rendered as premultiplied RGBA8; the encoder can flatten alpha over `bg_rgba`.
pub fn render_to_mp4(
    comp: &Composition,
    out_path: impl Into<std::path::PathBuf>,
    opts: RenderToMp4Opts,
    backend: &mut dyn RenderBackend,
    assets: &PreparedAssetStore,
) -> WavyteResult<()> {
    let _ = render_to_mp4_with_stats(comp, out_path, opts, backend, assets)?;
    Ok(())
}

/// Render a frame range to MP4 and return rendering stats.
pub fn render_to_mp4_with_stats(
    comp: &Composition,
    out_path: impl Into<std::path::PathBuf>,
    opts: RenderToMp4Opts,
    backend: &mut dyn RenderBackend,
    assets: &PreparedAssetStore,
) -> WavyteResult<RenderStats> {
    comp.validate()?;
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
    if !crate::encode::ffmpeg::is_ffmpeg_on_path() {
        return Err(WavyteError::evaluation(
            "ffmpeg is required for MP4 rendering, but was not found on PATH",
        ));
    }

    let mut audio_tmp = TempFileGuard(None);
    let audio_manifest = crate::build_audio_manifest(comp, assets, opts.range)?;
    let audio_cfg = if audio_manifest.segments.is_empty() {
        None
    } else {
        let mixed = crate::mix_manifest(&audio_manifest);
        let path = std::env::temp_dir().join(format!(
            "wavyte_audio_mix_{}_{}.f32le",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        crate::write_mix_to_f32le_file(&mixed, &path)?;
        audio_tmp.0 = Some(path.clone());
        Some(crate::encode::ffmpeg::AudioInputConfig {
            path,
            sample_rate: audio_manifest.sample_rate,
            channels: audio_manifest.channels,
        })
    };

    let cfg = crate::encode::ffmpeg::EncodeConfig {
        width: comp.canvas.width,
        height: comp.canvas.height,
        fps,
        out_path,
        overwrite: opts.overwrite,
        audio: audio_cfg,
    };

    let mut enc = crate::encode::ffmpeg::FfmpegEncoder::new(cfg, opts.bg_rgba)?;
    let mut stats = RenderStats::default();
    let chunk_size = normalized_chunk_size(opts.threading.chunk_size);

    let mut maybe_pool = None;
    let mut maybe_worker_settings = None;
    let layout_offsets = crate::resolve_layout_offsets(comp, assets)?;
    let mut compile_cache = CompileCache::default();
    if opts.threading.parallel {
        maybe_pool = Some(build_thread_pool(opts.threading.threads)?);
        maybe_worker_settings = Some(backend.worker_render_settings().ok_or_else(|| {
            WavyteError::evaluation(
                "parallel render_to_mp4 requires backend worker settings support (CpuBackend)",
            )
        })?);
    }

    let mut chunk_start = opts.range.start.0;
    while chunk_start < opts.range.end.0 {
        let chunk_end = (chunk_start + chunk_size).min(opts.range.end.0);
        let chunk = FrameRange::new(FrameIndex(chunk_start), FrameIndex(chunk_end))
            .map_err(|e| WavyteError::evaluation(format!("invalid chunk range: {e}")))?;

        let chunk_out = if opts.threading.parallel {
            render_chunk_parallel_cpu_unique(
                comp,
                chunk,
                assets,
                maybe_worker_settings
                    .as_ref()
                    .expect("worker settings present when parallel"),
                &opts.threading,
                maybe_pool.as_ref().expect("pool present when parallel"),
                &layout_offsets,
            )?
        } else {
            let (frames, stats_chunk) = render_chunk_sequential(
                comp,
                chunk,
                backend,
                assets,
                &layout_offsets,
                &mut compile_cache,
            )?;
            let frame_count = frames.len();
            ChunkParallelOut {
                unique_frames: frames,
                frame_to_unique: (0..frame_count).collect(),
                stats: stats_chunk,
            }
        };

        for &u in &chunk_out.frame_to_unique {
            enc.encode_frame(chunk_out.unique_frames.get(u).ok_or_else(|| {
                WavyteError::evaluation(
                    "internal error: unique frame index out of range during encode",
                )
            })?)?;
        }

        stats.frames_total += chunk_out.stats.frames_total;
        stats.frames_rendered += chunk_out.stats.frames_rendered;
        stats.frames_elided += chunk_out.stats.frames_elided;
        chunk_start = chunk_end;
    }

    enc.finish()?;
    drop(audio_tmp);
    Ok(stats)
}

fn render_chunk_sequential(
    comp: &Composition,
    range: FrameRange,
    backend: &mut dyn RenderBackend,
    assets: &PreparedAssetStore,
    layout_offsets: &crate::LayoutOffsets,
    compile_cache: &mut CompileCache,
) -> WavyteResult<(Vec<FrameRGBA>, RenderStats)> {
    let mut out = Vec::with_capacity(range.len_frames() as usize);
    for f in range.start.0..range.end.0 {
        let eval =
            Evaluator::eval_frame_with_layout_unchecked(comp, FrameIndex(f), layout_offsets)?;
        let plan = compile_frame_with_cache(comp, &eval, assets, compile_cache)?;
        out.push(execute_plan(backend, &plan, assets)?);
    }
    let total = range.len_frames();
    Ok((
        out,
        RenderStats {
            frames_total: total,
            frames_rendered: total,
            frames_elided: 0,
        },
    ))
}

struct ChunkParallelOut {
    unique_frames: Vec<FrameRGBA>,
    frame_to_unique: Vec<usize>,
    stats: RenderStats,
}

fn render_chunk_parallel_cpu_unique(
    comp: &Composition,
    range: FrameRange,
    assets: &PreparedAssetStore,
    settings: &RenderSettings,
    threading: &RenderThreading,
    pool: &rayon::ThreadPool,
    layout_offsets: &crate::LayoutOffsets,
) -> WavyteResult<ChunkParallelOut> {
    let mut evals = Vec::with_capacity(range.len_frames() as usize);
    for f in range.start.0..range.end.0 {
        evals.push(Evaluator::eval_frame_with_layout_unchecked(
            comp,
            FrameIndex(f),
            layout_offsets,
        )?);
    }

    let mut unique_indices = Vec::<usize>::with_capacity(evals.len());
    let mut frame_to_unique = Vec::<usize>::with_capacity(evals.len());
    if threading.static_frame_elision {
        let mut first = HashMap::<FrameFingerprint, usize>::new();
        for (idx, eval) in evals.iter().enumerate() {
            let fingerprint = fingerprint_eval(eval);
            if let Some(existing) = first.get(&fingerprint).copied() {
                frame_to_unique.push(existing);
            } else {
                let slot = unique_indices.len();
                unique_indices.push(idx);
                first.insert(fingerprint, slot);
                frame_to_unique.push(slot);
            }
        }
    } else {
        for idx in 0..evals.len() {
            frame_to_unique.push(idx);
            unique_indices.push(idx);
        }
    }

    let rendered = pool.install(|| {
        unique_indices
            .par_iter()
            .map_init(
                || {
                    (
                        crate::render::cpu::CpuBackend::new(settings.clone()),
                        CompileCache::default(),
                    )
                },
                |(worker_backend, worker_compile_cache), eval_idx| -> WavyteResult<FrameRGBA> {
                    let eval = &evals[*eval_idx];
                    let plan = compile_frame_with_cache(comp, eval, assets, worker_compile_cache)?;
                    worker_backend.render_plan(&plan, assets)
                },
            )
            .collect::<Vec<_>>()
    });

    let mut unique_frames = Vec::<FrameRGBA>::with_capacity(rendered.len());
    for item in rendered {
        unique_frames.push(item?);
    }

    let total = evals.len() as u64;
    let rendered_count = unique_indices.len() as u64;
    Ok(ChunkParallelOut {
        unique_frames,
        frame_to_unique,
        stats: RenderStats {
            frames_total: total,
            frames_rendered: rendered_count,
            frames_elided: total.saturating_sub(rendered_count),
        },
    })
}

fn render_chunk_parallel_cpu(
    comp: &Composition,
    range: FrameRange,
    assets: &PreparedAssetStore,
    settings: &RenderSettings,
    threading: &RenderThreading,
    pool: &rayon::ThreadPool,
    layout_offsets: &crate::LayoutOffsets,
) -> WavyteResult<(Vec<FrameRGBA>, RenderStats)> {
    let chunk_out = render_chunk_parallel_cpu_unique(
        comp,
        range,
        assets,
        settings,
        threading,
        pool,
        layout_offsets,
    )?;

    let mut unique_frames = chunk_out
        .unique_frames
        .into_iter()
        .map(Some)
        .collect::<Vec<_>>();
    let mut remaining = vec![0usize; unique_frames.len()];
    for &u in &chunk_out.frame_to_unique {
        remaining[u] += 1;
    }

    let mut out = Vec::<FrameRGBA>::with_capacity(chunk_out.frame_to_unique.len());
    for u in chunk_out.frame_to_unique {
        if remaining[u] == 1 {
            out.push(unique_frames[u].take().ok_or_else(|| {
                WavyteError::evaluation("internal error: unique frame missing at final take")
            })?);
        } else {
            out.push(
                unique_frames[u]
                    .as_ref()
                    .ok_or_else(|| {
                        WavyteError::evaluation(
                            "internal error: unique frame missing during clone path",
                        )
                    })?
                    .clone(),
            );
        }
        remaining[u] -= 1;
    }
    Ok((out, chunk_out.stats))
}

fn build_thread_pool(threads: Option<usize>) -> WavyteResult<rayon::ThreadPool> {
    if let Some(n) = threads
        && n == 0
    {
        return Err(WavyteError::validation(
            "render threading 'threads' must be >= 1 when set",
        ));
    }

    let mut builder = rayon::ThreadPoolBuilder::new();
    if let Some(n) = threads {
        builder = builder.num_threads(n);
    }
    builder
        .build()
        .map_err(|e| WavyteError::evaluation(format!("failed to build rayon thread pool: {e}")))
}

fn normalized_chunk_size(chunk_size: usize) -> u64 {
    if chunk_size == 0 {
        1
    } else {
        chunk_size as u64
    }
}

struct TempFileGuard(Option<std::path::PathBuf>);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}
