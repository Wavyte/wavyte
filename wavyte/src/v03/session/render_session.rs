use crate::foundation::core::{FrameIndex, FrameRange};
use crate::foundation::error::{WavyteError, WavyteResult};
use crate::v03::audio::manifest::build_audio_manifest;
use crate::v03::audio::mix::{mix_manifest, write_mix_to_f32le_file};
use crate::v03::compile::compiler::compile_frame;
use crate::v03::compile::fingerprint::{FrameFingerprint, fingerprint_eval};
use crate::v03::encode::sink::{AudioInputConfig, FrameSink, SinkConfig};
use crate::v03::eval::evaluator::Evaluator;
use crate::v03::expression::compile::compile_expr_program;
use crate::v03::expression::program::ExprProgram;
use crate::v03::normalize::ir::NormalizedComposition;
use crate::v03::normalize::pass::normalize;
use crate::v03::render::backend::FrameRGBA;
use crate::v03::render::backend::RenderBackendV03;
use crate::v03::render::cpu::{CpuBackendOpts, CpuBackendV03};
use crate::v03::scene::composition::Composition;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};

const MAX_REORDER_BUFFER_BYTES: u64 = 128 * 1024 * 1024;

/// Options controlling `RenderSession` range rendering behavior.
#[derive(Clone, Debug)]
pub struct RenderSessionOpts {
    /// Enable frame-level parallelism (rayon), using a dedicated thread pool.
    pub parallel: bool,
    /// Chunk size used by the render->encode streaming pipeline.
    pub chunk_size: usize,
    /// Override the number of rayon worker threads. `None` uses rayon defaults.
    pub threads: Option<usize>,
    /// Enable static-frame elision (skip rendering duplicate evaluated frames within a chunk).
    pub static_frame_elision: bool,
    /// Bounded channel capacity between render workers and the encoder thread.
    pub channel_capacity: usize,
    /// Enable audio mixing for `render_range` (mixed once per range, outside the per-frame loop).
    pub enable_audio: bool,
}

impl Default for RenderSessionOpts {
    fn default() -> Self {
        Self {
            parallel: false,
            chunk_size: 64,
            threads: None,
            static_frame_elision: false,
            channel_capacity: 4,
            enable_audio: true,
        }
    }
}

/// Range render statistics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderStats {
    /// Total frames in the requested range.
    pub frames_total: u64,
    /// Frames actually rendered (may be < total when static-frame elision is enabled).
    pub frames_rendered: u64,
    /// Frames elided due to static-frame elision.
    pub frames_elided: u64,
}

/// Session-oriented renderer for v0.3 compositions.
///
/// A session front-loads normalization and expression compilation, then provides efficient per-frame
/// execution for single frames and ranges.
pub struct RenderSession {
    norm: NormalizedComposition,
    expr_program: ExprProgram,
    eval: Evaluator,

    assets_root: PathBuf,
    opts: RenderSessionOpts,
}

impl RenderSession {
    /// Construct a new v0.3 render session.
    pub fn new(
        comp: &Composition,
        assets_root: impl Into<PathBuf>,
        opts: RenderSessionOpts,
    ) -> WavyteResult<Self> {
        comp.validate()?;
        let def = comp.def();
        let norm = normalize(def).map_err(|e| WavyteError::validation(e.to_string()))?;
        let expr_program =
            compile_expr_program(&norm).map_err(|e| WavyteError::validation(e.to_string()))?;
        let eval = Evaluator::new(expr_program.clone());
        Ok(Self {
            norm,
            expr_program,
            eval,
            assets_root: assets_root.into(),
            opts,
        })
    }

    pub(crate) fn ir(&self) -> &crate::v03::normalize::ir::CompositionIR {
        &self.norm.ir
    }

    pub(crate) fn interner(&self) -> &crate::v03::normalize::intern::StringInterner {
        &self.norm.interner
    }

    /// Render a single frame using the built-in CPU backend.
    pub fn render_frame(
        &mut self,
        frame: FrameIndex,
        backend_opts: CpuBackendOpts,
    ) -> WavyteResult<FrameRGBA> {
        if frame.0 >= self.norm.ir.duration_frames {
            return Err(WavyteError::validation(
                "render_frame frame must be within composition duration",
            ));
        }

        let g = self
            .eval
            .eval_frame(&self.norm.ir, frame.0)
            .map_err(|e| WavyteError::evaluation(e.to_string()))?;
        let plan = compile_frame(&self.norm.ir, g);
        let mut backend = CpuBackendV03::new(self.assets_root.clone(), backend_opts);
        backend.render_plan(&self.norm.ir, &self.norm.interner, g, &plan)
    }

    /// Render a frame range and stream frames into a sink.
    ///
    /// The sink receives frames in strictly increasing frame index order. When `parallel` is
    /// enabled, out-of-order worker completion is deterministically reordered at the sink boundary
    /// (bounded channel backpressure).
    pub fn render_range(
        &mut self,
        range: FrameRange,
        backend_opts: CpuBackendOpts,
        sink: &mut dyn FrameSink,
    ) -> WavyteResult<RenderStats> {
        if range.is_empty() {
            return Err(WavyteError::validation(
                "render_range range must be non-empty",
            ));
        }
        if range.end.0 > self.norm.ir.duration_frames {
            return Err(WavyteError::validation(
                "render_range range must be within composition duration",
            ));
        }

        let mut audio_tmp = TempFileGuard(None);
        let audio_cfg = if self.opts.enable_audio {
            let manifest = build_audio_manifest(
                &self.norm.ir,
                &self.norm.interner,
                &self.assets_root,
                &self.expr_program,
                range,
            )?;
            if manifest.segments.is_empty() {
                None
            } else {
                let mixed = mix_manifest(&manifest);
                let path = std::env::temp_dir().join(format!(
                    "wavyte_v03_audio_mix_{}_{}.f32le",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or(0)
                ));
                write_mix_to_f32le_file(&mixed, &path)?;
                audio_tmp.0 = Some(path.clone());
                Some(AudioInputConfig {
                    path,
                    sample_rate: manifest.sample_rate,
                    channels: manifest.channels,
                })
            }
        } else {
            None
        };

        let cfg = SinkConfig {
            width: self.norm.ir.canvas.width,
            height: self.norm.ir.canvas.height,
            fps: self.norm.ir.fps,
            audio: audio_cfg,
        };

        let cap = self.opts.channel_capacity.max(1);
        let bytes_per_frame = (cfg.width as u64)
            .saturating_mul(cfg.height as u64)
            .saturating_mul(4)
            .max(1);
        let max_chunk_by_mem = (MAX_REORDER_BUFFER_BYTES / bytes_per_frame).max(1);

        let mut chunk_size = normalized_chunk_size(self.opts.chunk_size).min(max_chunk_by_mem);
        chunk_size = chunk_size.min(range.len_frames());

        let pool = if self.opts.parallel {
            Some(build_thread_pool(self.opts.threads)?)
        } else {
            None
        };

        // Encoder thread: enforce in-order delivery to the sink regardless of render completion
        // order.
        std::thread::scope(|scope| -> WavyteResult<RenderStats> {
            let (tx, rx) = mpsc::sync_channel::<FrameMsg>(cap);
            let cfg_enc = cfg.clone();
            let range_start = range.start.0;
            let range_end = range.end.0;
            let sink_ref: &mut dyn FrameSink = sink;

            let enc = scope.spawn(move || -> WavyteResult<()> {
                sink_ref.begin(cfg_enc)?;

                let mut next = range_start;
                let mut pending = HashMap::<u64, Arc<FrameRGBA>>::new();
                while next < range_end {
                    if let Some(frame) = pending.remove(&next) {
                        sink_ref.push_frame(FrameIndex(next), &frame)?;
                        next += 1;
                        continue;
                    }

                    let msg = rx.recv().map_err(|_| {
                        WavyteError::evaluation("encoder channel disconnected unexpectedly")
                    })?;
                    pending.insert(msg.idx.0, msg.frame);

                    while let Some(frame) = pending.remove(&next) {
                        sink_ref.push_frame(FrameIndex(next), &frame)?;
                        next += 1;
                        if next >= range_end {
                            break;
                        }
                    }
                }

                sink_ref.end()?;
                Ok(())
            });

            let mut stats = RenderStats::default();
            let produce_res = if let Some(pool) = pool.as_ref() {
                let mut chunk_start = range_start;
                while chunk_start < range_end {
                    let chunk_end = (chunk_start + chunk_size).min(range_end);
                    let base_ctx = ParallelChunkCtx {
                        ir: &self.norm.ir,
                        interner: &self.norm.interner,
                        expr_program: &self.expr_program,
                        assets_root: &self.assets_root,
                        backend_opts,
                        pool,
                    };
                    let send_ctx = ParallelChunkSendCtx {
                        base: base_ctx,
                        tx: &tx,
                    };
                    if self.opts.static_frame_elision {
                        let (unique_frames, frame_to_unique, chunk_stats) =
                            render_chunk_unique_with_elision(
                                &mut self.eval,
                                &base_ctx,
                                chunk_start,
                                chunk_end,
                            )?;
                        for (i, f) in (chunk_start..chunk_end).enumerate() {
                            let u = frame_to_unique[i];
                            tx.send(FrameMsg {
                                idx: FrameIndex(f),
                                frame: unique_frames[u].clone(),
                            })
                            .map_err(|_| {
                                WavyteError::evaluation("encoder thread is not accepting frames")
                            })?;
                        }
                        stats.frames_total += chunk_stats.frames_total;
                        stats.frames_rendered += chunk_stats.frames_rendered;
                        stats.frames_elided += chunk_stats.frames_elided;
                    } else {
                        render_chunk_send_no_elision(&send_ctx, chunk_start, chunk_end)?;
                        stats.frames_total += chunk_end - chunk_start;
                        stats.frames_rendered += chunk_end - chunk_start;
                    }

                    chunk_start = chunk_end;
                }
                Ok(())
            } else {
                let mut backend = CpuBackendV03::new(self.assets_root.clone(), backend_opts);
                let mut chunk_start = range_start;
                while chunk_start < range_end {
                    let chunk_end = (chunk_start + chunk_size).min(range_end);

                    if self.opts.static_frame_elision {
                        let mut cache = HashMap::<FrameFingerprint, Arc<FrameRGBA>>::new();
                        for f in chunk_start..chunk_end {
                            let g = self
                                .eval
                                .eval_frame(&self.norm.ir, f)
                                .map_err(|e| WavyteError::evaluation(e.to_string()))?;
                            let fp = fingerprint_eval(&self.norm.ir, g);
                            if let Some(frame) = cache.get(&fp) {
                                stats.frames_elided += 1;
                                tx.send(FrameMsg {
                                    idx: FrameIndex(f),
                                    frame: frame.clone(),
                                })
                                .map_err(|_| {
                                    WavyteError::evaluation(
                                        "encoder thread is not accepting frames",
                                    )
                                })?;
                                continue;
                            }

                            let plan = compile_frame(&self.norm.ir, g);
                            let frame = backend.render_plan(
                                &self.norm.ir,
                                &self.norm.interner,
                                g,
                                &plan,
                            )?;
                            let arc = Arc::new(frame);
                            cache.insert(fp, arc.clone());
                            stats.frames_rendered += 1;
                            tx.send(FrameMsg {
                                idx: FrameIndex(f),
                                frame: arc,
                            })
                            .map_err(|_| {
                                WavyteError::evaluation("encoder thread is not accepting frames")
                            })?;
                        }
                    } else {
                        for f in chunk_start..chunk_end {
                            let g = self
                                .eval
                                .eval_frame(&self.norm.ir, f)
                                .map_err(|e| WavyteError::evaluation(e.to_string()))?;
                            let plan = compile_frame(&self.norm.ir, g);
                            let frame = backend.render_plan(
                                &self.norm.ir,
                                &self.norm.interner,
                                g,
                                &plan,
                            )?;
                            stats.frames_rendered += 1;
                            tx.send(FrameMsg {
                                idx: FrameIndex(f),
                                frame: Arc::new(frame),
                            })
                            .map_err(|_| {
                                WavyteError::evaluation("encoder thread is not accepting frames")
                            })?;
                        }
                    }

                    stats.frames_total += chunk_end - chunk_start;
                    chunk_start = chunk_end;
                }
                Ok(())
            };

            drop(tx);
            let enc_res = enc
                .join()
                .map_err(|_| WavyteError::evaluation("encoder thread panicked"))?;

            let _ = audio_tmp;

            if let Err(e) = produce_res {
                let _ = enc_res;
                return Err(e);
            }
            enc_res?;
            Ok(stats)
        })
    }
}

#[derive(Debug)]
struct FrameMsg {
    idx: FrameIndex,
    frame: Arc<FrameRGBA>,
}

fn normalized_chunk_size(chunk_size: usize) -> u64 {
    if chunk_size == 0 {
        1
    } else {
        chunk_size as u64
    }
}

fn build_thread_pool(threads: Option<usize>) -> WavyteResult<rayon::ThreadPool> {
    if let Some(n) = threads
        && n == 0
    {
        return Err(WavyteError::validation(
            "render_range 'threads' must be >= 1 when set",
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

struct Worker {
    eval: Evaluator,
    backend: CpuBackendV03,
}

impl Worker {
    fn new(expr_program: ExprProgram, assets_root: PathBuf, backend_opts: CpuBackendOpts) -> Self {
        Self {
            eval: Evaluator::new(expr_program),
            backend: CpuBackendV03::new(assets_root, backend_opts),
        }
    }
}

#[derive(Clone, Copy)]
struct ParallelChunkCtx<'a> {
    ir: &'a crate::v03::normalize::ir::CompositionIR,
    interner: &'a crate::v03::normalize::intern::StringInterner,
    expr_program: &'a ExprProgram,
    assets_root: &'a Path,
    backend_opts: CpuBackendOpts,
    pool: &'a rayon::ThreadPool,
}

#[derive(Clone, Copy)]
struct ParallelChunkSendCtx<'a> {
    base: ParallelChunkCtx<'a>,
    tx: &'a mpsc::SyncSender<FrameMsg>,
}

fn render_chunk_unique_with_elision(
    eval_main: &mut Evaluator,
    ctx: &ParallelChunkCtx<'_>,
    start: u64,
    end: u64,
) -> WavyteResult<(Vec<Arc<FrameRGBA>>, Vec<usize>, RenderStats)> {
    let frames: Vec<u64> = (start..end).collect();
    let mut uniq = Vec::<u64>::new();
    let mut map = Vec::<usize>::with_capacity(frames.len());
    let mut seen = HashMap::<FrameFingerprint, usize>::new();

    for &f in &frames {
        let g = eval_main
            .eval_frame(ctx.ir, f)
            .map_err(|e| WavyteError::evaluation(e.to_string()))?;
        let fp = fingerprint_eval(ctx.ir, g);
        let u = *seen.entry(fp).or_insert_with(|| {
            let i = uniq.len();
            uniq.push(f);
            i
        });
        map.push(u);
    }

    let rendered = ctx.pool.install(|| {
        uniq.par_iter()
            .enumerate()
            .map_init(
                || {
                    Worker::new(
                        ctx.expr_program.clone(),
                        ctx.assets_root.to_path_buf(),
                        ctx.backend_opts,
                    )
                },
                |w, (i, &f)| -> WavyteResult<(usize, Arc<FrameRGBA>)> {
                    let g = w
                        .eval
                        .eval_frame(ctx.ir, f)
                        .map_err(|e| WavyteError::evaluation(e.to_string()))?;
                    let plan = compile_frame(ctx.ir, g);
                    let frame = w.backend.render_plan(ctx.ir, ctx.interner, g, &plan)?;
                    Ok((i, Arc::new(frame)))
                },
            )
            .collect::<Vec<_>>()
    });

    let mut unique_frames = vec![None::<Arc<FrameRGBA>>; uniq.len()];
    for r in rendered {
        let (i, frame) = r?;
        unique_frames[i] = Some(frame);
    }

    let unique_frames = unique_frames
        .into_iter()
        .map(|x| x.ok_or_else(|| WavyteError::evaluation("missing unique rendered frame")))
        .collect::<WavyteResult<Vec<_>>>()?;

    let total = end - start;
    let rendered_count = unique_frames.len() as u64;
    Ok((
        unique_frames,
        map,
        RenderStats {
            frames_total: total,
            frames_rendered: rendered_count,
            frames_elided: total.saturating_sub(rendered_count),
        },
    ))
}

fn render_chunk_send_no_elision(
    ctx: &ParallelChunkSendCtx<'_>,
    start: u64,
    end: u64,
) -> WavyteResult<()> {
    let tx = ctx.tx.clone();
    ctx.base.pool.install(|| {
        (start..end).into_par_iter().try_for_each_init(
            || {
                Worker::new(
                    ctx.base.expr_program.clone(),
                    ctx.base.assets_root.to_path_buf(),
                    ctx.base.backend_opts,
                )
            },
            move |w, f| -> WavyteResult<()> {
                let g = w
                    .eval
                    .eval_frame(ctx.base.ir, f)
                    .map_err(|e| WavyteError::evaluation(e.to_string()))?;
                let plan = compile_frame(ctx.base.ir, g);
                let frame = w
                    .backend
                    .render_plan(ctx.base.ir, ctx.base.interner, g, &plan)?;
                tx.send(FrameMsg {
                    idx: FrameIndex(f),
                    frame: Arc::new(frame),
                })
                .map_err(|_| WavyteError::evaluation("encoder thread is not accepting frames"))?;
                Ok(())
            },
        )
    })
}

struct TempFileGuard(Option<PathBuf>);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v03::animation::anim::{AnimDef, AnimTaggedDef};
    use crate::v03::encode::sink::InMemorySink;
    use crate::v03::scene::composition::Composition;
    use crate::v03::scene::model::{
        AssetDef, CanvasDef, CompositionDef, FpsDef, NodeDef, NodeKindDef,
    };
    use std::collections::BTreeMap;

    fn make_comp(duration: u64, opacity: AnimDef<f64>) -> CompositionDef {
        let mut assets = BTreeMap::new();
        assets.insert("solid".to_owned(), AssetDef::SolidRect { color: None });

        CompositionDef {
            version: "0.3".to_owned(),
            canvas: CanvasDef {
                width: 32,
                height: 32,
            },
            fps: FpsDef { num: 30, den: 1 },
            duration,
            seed: 0,
            variables: BTreeMap::new(),
            assets,
            root: NodeDef {
                id: "root".to_owned(),
                kind: NodeKindDef::Leaf {
                    asset: "solid".to_owned(),
                },
                range: [0, duration],
                transform: Default::default(),
                opacity,
                layout: None,
                effects: Vec::new(),
                mask: None,
                transition_in: None,
                transition_out: None,
            },
        }
    }

    #[test]
    fn render_range_parallel_inmemory_is_ordered_and_varies_by_frame() {
        let comp = Composition::from_def(make_comp(
            8,
            AnimDef::Tagged(AnimTaggedDef::Expr("=time.progress".to_owned())),
        ));
        let mut sess = RenderSession::new(
            &comp,
            std::env::temp_dir(),
            RenderSessionOpts {
                parallel: true,
                chunk_size: 1024,
                threads: Some(2),
                static_frame_elision: false,
                channel_capacity: 4,
                enable_audio: false,
            },
        )
        .unwrap();

        let mut sink = InMemorySink::new();
        let stats = sess
            .render_range(
                FrameRange {
                    start: FrameIndex(0),
                    end: FrameIndex(8),
                },
                CpuBackendOpts::default(),
                &mut sink,
            )
            .unwrap();

        assert_eq!(
            stats,
            RenderStats {
                frames_total: 8,
                frames_rendered: 8,
                frames_elided: 0,
            }
        );
        assert_eq!(sink.frames.len(), 8);
        for (i, (idx, _)) in sink.frames.iter().enumerate() {
            assert_eq!(idx.0, i as u64);
        }

        let a = &sink.frames[0].1.data;
        let b = &sink.frames[1].1.data;
        assert_ne!(a, b, "expected frame-to-frame variation");
    }

    #[test]
    fn render_range_static_frame_elision_reuses_payloads() {
        let comp = Composition::from_def(make_comp(8, AnimDef::Constant(1.0)));
        let mut sess = RenderSession::new(
            &comp,
            std::env::temp_dir(),
            RenderSessionOpts {
                parallel: true,
                chunk_size: 1024,
                threads: Some(2),
                static_frame_elision: true,
                channel_capacity: 4,
                enable_audio: false,
            },
        )
        .unwrap();

        let mut sink = InMemorySink::new();
        let stats = sess
            .render_range(
                FrameRange {
                    start: FrameIndex(0),
                    end: FrameIndex(8),
                },
                CpuBackendOpts::default(),
                &mut sink,
            )
            .unwrap();

        assert_eq!(stats.frames_total, 8);
        assert_eq!(stats.frames_rendered, 1);
        assert_eq!(stats.frames_elided, 7);

        assert_eq!(sink.frames.len(), 8);
        for (i, (idx, frame)) in sink.frames.iter().enumerate() {
            assert_eq!(idx.0, i as u64);
            assert_eq!(
                &frame.data, &sink.frames[0].1.data,
                "expected all frames to reuse identical payload"
            );
        }
    }

    #[test]
    fn render_range_parallel_matches_sequential_output() {
        let comp = Composition::from_def(make_comp(
            8,
            AnimDef::Tagged(AnimTaggedDef::Expr("=time.progress".to_owned())),
        ));
        let range = FrameRange {
            start: FrameIndex(0),
            end: FrameIndex(8),
        };

        let mut sess_seq = RenderSession::new(
            &comp,
            std::env::temp_dir(),
            RenderSessionOpts {
                parallel: false,
                chunk_size: 1024,
                threads: None,
                static_frame_elision: false,
                channel_capacity: 4,
                enable_audio: false,
            },
        )
        .unwrap();
        let mut sink_seq = InMemorySink::new();
        sess_seq
            .render_range(range, CpuBackendOpts::default(), &mut sink_seq)
            .unwrap();

        let mut sess_par = RenderSession::new(
            &comp,
            std::env::temp_dir(),
            RenderSessionOpts {
                parallel: true,
                chunk_size: 1024,
                threads: Some(2),
                static_frame_elision: false,
                channel_capacity: 4,
                enable_audio: false,
            },
        )
        .unwrap();
        let mut sink_par = InMemorySink::new();
        sess_par
            .render_range(range, CpuBackendOpts::default(), &mut sink_par)
            .unwrap();

        assert_eq!(sink_seq.frames.len(), sink_par.frames.len());
        for ((idx_a, a), (idx_b, b)) in sink_seq.frames.iter().zip(sink_par.frames.iter()) {
            assert_eq!(idx_a, idx_b);
            assert_eq!(a.width, b.width);
            assert_eq!(a.height, b.height);
            assert_eq!(a.data, b.data);
        }
    }
}
