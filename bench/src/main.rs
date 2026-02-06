use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::Context as _;
use serde_json::json;
use sha2::Digest as _;

#[derive(Clone, Debug)]
struct BenchArgs {
    width: u32,
    height: u32,
    fps: u32,
    seconds: u32,
    warmup: u32,
    repeats: u32,
    backend: Backend,
    out_dir: PathBuf,
    keep_all_outputs: bool,
    no_encode: bool,
    blur_radius: u32,
    parallel: bool,
    threads: Option<usize>,
    chunk_size: usize,
    static_frame_elision: bool,
}

#[derive(Clone, Debug)]
struct SceneAssets {
    font_rel: String,
    svg_rel: String,
    image_rel: String,
}

#[derive(Clone, Debug)]
struct SceneParams {
    width: u32,
    height: u32,
    fps: u32,
    duration_frames: u64,
    blur_radius: u32,
}

#[derive(Clone, Copy, Debug)]
enum Backend {
    Cpu,
}

#[derive(Clone, Debug, Default)]
struct RunMetrics {
    backend_create: Duration,
    ffmpeg_spawn: Duration,
    eval_total: Duration,
    compile_total: Duration,
    render_total: Duration,
    encode_write_total: Duration,
    ffmpeg_finish: Duration,
    wall_total: Duration,
}

fn main() {
    if let Err(err) = try_main() {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}

fn try_main() -> anyhow::Result<()> {
    let args = parse_args()?;

    if args.width == 0 || args.height == 0 {
        anyhow::bail!("--width/--height must be > 0");
    }
    if !args.width.is_multiple_of(2) || !args.height.is_multiple_of(2) {
        anyhow::bail!("--width/--height must be even (required by the MP4 encoder defaults)");
    }
    if args.fps == 0 || args.seconds == 0 {
        anyhow::bail!("--fps and --seconds must be > 0");
    }
    if args.chunk_size == 0 {
        anyhow::bail!("--chunk-size must be >= 1");
    }
    if let Some(n) = args.threads
        && n == 0
    {
        anyhow::bail!("--threads must be >= 1 when set");
    }

    let bench_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = bench_root
        .parent()
        .ok_or_else(|| anyhow::anyhow!("bench manifest dir has no parent (unexpected)"))?
        .to_path_buf();

    let assets_dir = repo_root.join("assets");

    let (font_rel, svg_rel, image_rel) = select_assets(&assets_dir)?;
    eprintln!("assets:");
    eprintln!("  font:  {font_rel}");
    eprintln!("  svg:   {svg_rel}");
    eprintln!("  image: {image_rel}");

    let frames = u64::from(args.fps) * u64::from(args.seconds);
    let assets = SceneAssets {
        font_rel,
        svg_rel,
        image_rel,
    };
    let params = SceneParams {
        width: args.width,
        height: args.height,
        fps: args.fps,
        duration_frames: frames,
        blur_radius: args.blur_radius,
    };
    let comp = build_benchmark_comp(&params, &assets)?;

    dump_font_diagnostics(&comp, &repo_root)?;

    let out_dir = if args.out_dir.is_absolute() {
        args.out_dir.clone()
    } else {
        repo_root.join(&args.out_dir)
    };
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("create out dir '{}'", out_dir.display()))?;

    if !args.no_encode && !wavyte::is_ffmpeg_on_path() {
        anyhow::bail!("ffmpeg is required for encoding; install it and ensure it's on PATH");
    }

    if args.warmup > 0 {
        eprintln!("warmup: {} run(s)", args.warmup);
        for i in 0..args.warmup {
            let _ = run_once(
                &args, &repo_root, &out_dir, &comp, i, /*is_warmup=*/ true,
            )?;
        }
    }

    eprintln!(
        "bench: {repeats} run(s) ({profile} build), {frames} frames/run ({seconds}s @ {fps} fps), backend={backend:?}, encode={encode}, mode={mode}, threads={threads}, chunk={chunk}, elision={elision}",
        repeats = args.repeats,
        profile = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        frames = frames,
        seconds = args.seconds,
        fps = args.fps,
        backend = args.backend,
        encode = if args.no_encode { "no" } else { "yes" },
        mode = if args.parallel {
            "parallel"
        } else {
            "sequential"
        },
        threads = args
            .threads
            .map(|n| n.to_string())
            .unwrap_or_else(|| "auto".to_string()),
        chunk = args.chunk_size,
        elision = if args.static_frame_elision {
            "on"
        } else {
            "off"
        },
    );

    let mut runs = Vec::<RunMetrics>::with_capacity(args.repeats as usize);
    for i in 0..args.repeats {
        runs.push(run_once(
            &args, &repo_root, &out_dir, &comp, i, /*is_warmup=*/ false,
        )?);
    }

    report_percentiles(&runs);
    Ok(())
}

fn dump_font_diagnostics(comp: &wavyte::Composition, repo_root: &Path) -> anyhow::Result<()> {
    let assets = wavyte::PreparedAssetStore::prepare(comp, repo_root)?;

    eprintln!("font diagnostics:");
    for (key, asset) in &comp.assets {
        match asset {
            wavyte::Asset::Text(a) => {
                let id = assets
                    .id_for_key(key)
                    .with_context(|| format!("resolve text asset id '{key}'"))?;
                let prepared = assets
                    .get(id)
                    .with_context(|| format!("load text asset '{key}'"))?;
                let wavyte::PreparedAsset::Text(p) = prepared else {
                    anyhow::bail!("text asset '{key}' did not prepare as text (bug)");
                };
                eprintln!("  text:{key}:");
                eprintln!("    font_source: {}", a.font_source);
                eprintln!("    family:      {}", p.font_family);
                eprintln!("    sha256:      {}", sha256_hex(&p.font_bytes));
            }
            wavyte::Asset::Svg(a) => {
                let id = assets
                    .id_for_key(key)
                    .with_context(|| format!("resolve svg asset id '{key}'"))?;
                let prepared = assets
                    .get(id)
                    .with_context(|| format!("load svg asset '{key}'"))?;
                let wavyte::PreparedAsset::Svg(p) = prepared else {
                    anyhow::bail!("svg asset '{key}' did not prepare as svg (bug)");
                };
                eprintln!("  svg:{key}:");
                eprintln!("    source:     {}", a.source);
                eprintln!("    font_faces: {}", p.tree.fontdb().faces().count());
            }
            _ => {}
        }
    }

    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = sha2::Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

fn parse_args() -> anyhow::Result<BenchArgs> {
    let mut args = std::env::args().skip(1);

    let mut out = BenchArgs {
        width: 640,
        height: 360,
        fps: 30,
        seconds: 10,
        warmup: 1,
        repeats: 100,
        backend: Backend::Cpu,
        out_dir: PathBuf::from("assets/bench"),
        keep_all_outputs: false,
        no_encode: false,
        blur_radius: 0,
        parallel: false,
        threads: None,
        chunk_size: 64,
        static_frame_elision: false,
    };

    while let Some(a) = args.next() {
        match a.as_str() {
            "--width" => out.width = parse_u32(args.next(), "--width")?,
            "--height" => out.height = parse_u32(args.next(), "--height")?,
            "--fps" => out.fps = parse_u32(args.next(), "--fps")?,
            "--seconds" => out.seconds = parse_u32(args.next(), "--seconds")?,
            "--warmup" => out.warmup = parse_u32(args.next(), "--warmup")?,
            "--repeats" => out.repeats = parse_u32(args.next(), "--repeats")?,
            "--blur-radius" => out.blur_radius = parse_u32(args.next(), "--blur-radius")?,
            "--out-dir" => {
                out.out_dir = PathBuf::from(args.next().ok_or_else(|| {
                    anyhow::anyhow!("missing value for --out-dir (expected a path)")
                })?)
            }
            "--backend" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("missing value for --backend (cpu)"))?;
                out.backend = match v.as_str() {
                    "cpu" => Backend::Cpu,
                    _ => anyhow::bail!("unknown --backend '{v}' (expected cpu)"),
                };
            }
            "--keep-all" => out.keep_all_outputs = true,
            "--no-encode" => out.no_encode = true,
            "--parallel" => out.parallel = true,
            "--threads" => out.threads = Some(parse_usize(args.next(), "--threads")?),
            "--chunk-size" => out.chunk_size = parse_usize(args.next(), "--chunk-size")?,
            "--static-frame-elision" => out.static_frame_elision = true,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            _ => anyhow::bail!("unknown arg '{a}' (try --help)"),
        }
    }

    Ok(out)
}

fn print_help() {
    eprintln!(
        r#"wavyte-bench (debug)

Renders a 10s composition repeatedly and reports p50/p90/p99 for each stage.

Usage:
  cargo run -q
  cargo run -q -- --repeats 100 --seconds 10 --fps 30
  cargo run -q -- --backend cpu
  cargo run -q -- --parallel --threads 2

Args:
  --width N        (default 640; must be even for MP4)
  --height N       (default 360;  must be even for MP4)
  --fps N          (default 30)
  --seconds N      (default 10)
  --warmup N       (default 1)
  --repeats N      (default 100)
  --blur-radius N  (default 0; 0 disables blur)
  --backend cpu (default cpu)
  --out-dir PATH   (default assets/bench)
  --keep-all       keep per-run outputs (otherwise overwrite the same file)
  --no-encode      render frames but do not spawn ffmpeg
  --parallel       use frame-parallel pipeline for eval+compile+render
  --threads N      worker threads for parallel mode (default auto)
  --chunk-size N   frames per chunk in parallel mode (default 64)
  --static-frame-elision  enable fingerprint-based still-frame elision
"#
    );
}

fn parse_u32(v: Option<String>, flag: &str) -> anyhow::Result<u32> {
    let v = v.ok_or_else(|| anyhow::anyhow!("missing value for {flag}"))?;
    v.parse::<u32>()
        .with_context(|| format!("parse {flag} value '{v}'"))
}

fn parse_usize(v: Option<String>, flag: &str) -> anyhow::Result<usize> {
    let v = v.ok_or_else(|| anyhow::anyhow!("missing value for {flag}"))?;
    v.parse::<usize>()
        .with_context(|| format!("parse {flag} value '{v}'"))
}

fn select_assets(assets_dir: &Path) -> anyhow::Result<(String, String, String)> {
    if !assets_dir.is_dir() {
        anyhow::bail!(
            "assets dir '{}' does not exist (expected repo-local assets/)",
            assets_dir.display()
        );
    }

    let font = find_first_file_with_ext(assets_dir, &["ttf", "otf"], &["out", "bench"])
        .context("select font (.ttf/.otf) from assets/")?;
    let svg = find_first_file_with_ext(assets_dir, &["svg"], &["out", "bench"])
        .context("select svg (.svg) from assets/")?;
    let image = find_first_file_with_ext(assets_dir, &["jpg", "jpeg"], &["out", "bench"])
        .or_else(|_| find_first_file_with_ext(assets_dir, &["png"], &["out", "bench"]))
        .context("select image (.jpg/.jpeg/.png) from assets/")?;

    Ok((
        format!("assets/{}", font),
        format!("assets/{}", svg),
        format!("assets/{}", image),
    ))
}

fn find_first_file_with_ext(
    dir: &Path,
    exts: &[&str],
    ignore_prefixes: &[&str],
) -> anyhow::Result<String> {
    let mut entries = Vec::<String>::new();
    for e in std::fs::read_dir(dir).with_context(|| format!("read_dir '{}'", dir.display()))? {
        let e = e?;
        let path = e.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if ignore_prefixes
            .iter()
            .any(|p| name.to_ascii_lowercase().starts_with(p))
        {
            continue;
        }
        let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
            continue;
        };
        let ext = ext.to_ascii_lowercase();
        if exts.iter().any(|e| *e == ext) {
            entries.push(name.to_string());
        }
    }
    entries.sort();
    entries.into_iter().next().ok_or_else(|| {
        anyhow::anyhow!(
            "no file with extensions {exts:?} found in '{}'",
            dir.display()
        )
    })
}

fn build_benchmark_comp(
    params: &SceneParams,
    assets: &SceneAssets,
) -> anyhow::Result<wavyte::Composition> {
    let duration = wavyte::FrameIndex(params.duration_frames);
    let fps = wavyte::Fps::new(params.fps, 1)?;

    let bg_path = format!(
        "M0,0 L{w},0 L{w},{h} L0,{h} Z",
        w = params.width,
        h = params.height
    );

    let comp = wavyte::CompositionBuilder::new(
        fps,
        wavyte::Canvas {
            width: params.width,
            height: params.height,
        },
        duration,
    )
    .seed(1)
    .asset(
        "bg",
        wavyte::Asset::Path(wavyte::PathAsset {
            svg_path_d: bg_path,
        }),
    )?
    .asset(
        "img",
        wavyte::Asset::Image(wavyte::ImageAsset {
            source: assets.image_rel.clone(),
        }),
    )?
    .asset(
        "svg",
        wavyte::Asset::Svg(wavyte::SvgAsset {
            source: assets.svg_rel.clone(),
        }),
    )?
    .asset(
        "txt",
        wavyte::Asset::Text(wavyte::TextAsset {
            text: "wavyte v0.1.0 benchmark".to_string(),
            font_source: assets.font_rel.clone(),
            size_px: 64.0,
            max_width_px: Some(params.width as f32 - 80.0),
            color_rgba8: [255, 255, 255, 255],
        }),
    )?
    .asset(
        "tri_a",
        wavyte::Asset::Path(wavyte::PathAsset {
            svg_path_d: "M60,0 L120,120 L0,120 Z".to_string(),
        }),
    )?
    .asset(
        "tri_b",
        wavyte::Asset::Path(wavyte::PathAsset {
            svg_path_d: "M0,0 L120,0 L60,120 Z".to_string(),
        }),
    )?
    .track(build_track(
        duration,
        params.width,
        params.height,
        params.blur_radius,
    )?) // track order is stable; eval sets painter order
    .build()?;

    Ok(comp)
}

fn build_track(
    duration: wavyte::FrameIndex,
    width: u32,
    height: u32,
    blur_radius: u32,
) -> anyhow::Result<wavyte::Track> {
    let full = wavyte::FrameRange::new(wavyte::FrameIndex(0), duration)?;
    let half = wavyte::FrameRange::new(wavyte::FrameIndex(duration.0 / 2), duration)?;

    let w = width as f64;
    let h = height as f64;

    let tr = wavyte::TransitionSpec {
        kind: "crossfade".to_string(),
        duration_frames: 30,
        ease: wavyte::Ease::Linear,
        params: serde_json::Value::Null,
    };

    let bg = wavyte::ClipBuilder::new("bg", "bg", full).build()?;

    let svg = wavyte::ClipBuilder::new("svg", "svg", full)
        .transform(wavyte::Anim::constant(wavyte::Transform2D {
            translate: wavyte::Vec2::new(w * 0.10, h * 0.10),
            scale: wavyte::Vec2::new(1.0, 1.0),
            ..wavyte::Transform2D::default()
        }))
        .build()?;

    let mut img = wavyte::ClipBuilder::new("img", "img", full).transform(wavyte::Anim::constant(
        wavyte::Transform2D {
            translate: wavyte::Vec2::new(w * 0.55, h * 0.15),
            scale: wavyte::Vec2::new(0.65, 0.65),
            ..wavyte::Transform2D::default()
        },
    ));
    if blur_radius > 0 {
        img = img.effect(wavyte::EffectInstance {
            kind: "blur".to_string(),
            params: json!({ "radius_px": blur_radius }),
        });
    }
    let img = img.build()?;

    let text = wavyte::ClipBuilder::new("txt", "txt", full)
        .transform(wavyte::Anim::constant(wavyte::Transform2D {
            translate: wavyte::Vec2::new(24.0, (h - 88.0).max(0.0)),
            ..wavyte::Transform2D::default()
        }))
        .build()?;

    // Exercise transition compilation without depending on external assets.
    let tri_a = wavyte::ClipBuilder::new("tri_a", "tri_a", full)
        .transform(wavyte::Anim::constant(wavyte::Transform2D {
            translate: wavyte::Vec2::new(w * 0.78, h * 0.55),
            scale: wavyte::Vec2::new(3.0, 3.0),
            ..wavyte::Transform2D::default()
        }))
        .transition_out(tr.clone())
        .build()?;

    let tri_b = wavyte::ClipBuilder::new("tri_b", "tri_b", half)
        .transform(wavyte::Anim::constant(wavyte::Transform2D {
            translate: wavyte::Vec2::new(w * 0.78, h * 0.55),
            scale: wavyte::Vec2::new(3.0, 3.0),
            ..wavyte::Transform2D::default()
        }))
        .transition_in(tr)
        .build()?;

    let track = wavyte::TrackBuilder::new("main")
        .clip(bg)
        .clip(svg)
        .clip(img)
        .clip(text)
        .clip(tri_a)
        .clip(tri_b)
        .build()?;

    Ok(track)
}

fn run_once(
    args: &BenchArgs,
    repo_root: &Path,
    out_dir: &Path,
    comp: &wavyte::Composition,
    run_idx: u32,
    is_warmup: bool,
) -> anyhow::Result<RunMetrics> {
    let wall = Instant::now();

    let backend_create_t0 = Instant::now();
    let settings = wavyte::RenderSettings {
        clear_rgba: Some([18, 20, 28, 255]),
    };
    let kind = match args.backend {
        Backend::Cpu => wavyte::BackendKind::Cpu,
    };
    let mut backend = wavyte::create_backend(kind, &settings)?;
    let backend_create = backend_create_t0.elapsed();

    let assets = wavyte::PreparedAssetStore::prepare(comp, repo_root)?;

    let mut enc = if args.no_encode {
        None
    } else {
        let out_path = if args.keep_all_outputs {
            out_dir.join(format!("out_{run_idx:03}.mp4"))
        } else {
            out_dir.join("out.mp4")
        };

        let spawn_t0 = Instant::now();
        let cfg = wavyte::default_mp4_config(
            out_path,
            comp.canvas.width,
            comp.canvas.height,
            comp.fps.num,
        );
        let enc = wavyte::FfmpegEncoder::new(cfg, [18, 20, 28, 255])?;
        let ffmpeg_spawn = spawn_t0.elapsed();
        Some((enc, ffmpeg_spawn))
    };

    let mut m = RunMetrics {
        backend_create,
        ffmpeg_spawn: enc.as_ref().map_or(Duration::ZERO, |(_, s)| *s),
        ..RunMetrics::default()
    };

    if args.parallel {
        let threading = wavyte::RenderThreading {
            parallel: true,
            chunk_size: args.chunk_size,
            threads: args.threads,
            static_frame_elision: args.static_frame_elision,
        };

        let range = wavyte::FrameRange::new(wavyte::FrameIndex(0), comp.duration)?;
        let t2 = Instant::now();
        let (frames, _stats) =
            wavyte::render_frames_with_stats(comp, range, backend.as_mut(), &assets, &threading)?;
        m.render_total += t2.elapsed();

        if let Some((enc, _spawn)) = enc.as_mut() {
            for frame in &frames {
                let t3 = Instant::now();
                enc.encode_frame(frame)?;
                m.encode_write_total += t3.elapsed();
            }
        }
    } else {
        for f in 0..comp.duration.0 {
            let t0 = Instant::now();
            let eval = wavyte::Evaluator::eval_frame(comp, wavyte::FrameIndex(f))?;
            m.eval_total += t0.elapsed();

            let t1 = Instant::now();
            let plan = wavyte::compile_frame(comp, &eval, &assets)?;
            m.compile_total += t1.elapsed();

            let t2 = Instant::now();
            let frame = backend.render_plan(&plan, &assets)?;
            m.render_total += t2.elapsed();

            if let Some((enc, _spawn)) = enc.as_mut() {
                let t3 = Instant::now();
                enc.encode_frame(&frame)?;
                m.encode_write_total += t3.elapsed();
            }
        }
    }

    if let Some((enc, _spawn)) = enc.take() {
        let t = Instant::now();
        enc.finish()?;
        m.ffmpeg_finish = t.elapsed();
    }

    m.wall_total = wall.elapsed();

    if !is_warmup {
        eprintln!(
            "run {run_idx:03}: wall={wall:.3}s eval={ev:.3}s compile={co:.3}s render={re:.3}s encode_write={en:.3}s ffmpeg_spawn={sp:.3}s ffmpeg_finish={fi:.3}s mode={mode}",
            wall = m.wall_total.as_secs_f64(),
            ev = m.eval_total.as_secs_f64(),
            co = m.compile_total.as_secs_f64(),
            re = m.render_total.as_secs_f64(),
            en = m.encode_write_total.as_secs_f64(),
            sp = m.ffmpeg_spawn.as_secs_f64(),
            fi = m.ffmpeg_finish.as_secs_f64(),
            mode = if args.parallel {
                "parallel(render=eval+compile+render)"
            } else {
                "sequential"
            },
        );
    }

    Ok(m)
}

fn report_percentiles(runs: &[RunMetrics]) {
    type Getter = fn(&RunMetrics) -> Duration;
    type Field = (&'static str, Getter);

    fn collect(runs: &[RunMetrics], f: fn(&RunMetrics) -> Duration) -> Vec<Duration> {
        let mut v = runs.iter().map(f).collect::<Vec<_>>();
        v.sort_by_key(|d| d.as_nanos());
        v
    }

    fn p(v: &[Duration], p: f64) -> Duration {
        if v.is_empty() {
            return Duration::ZERO;
        }
        let n = v.len();
        let rank = (p * (n as f64)).ceil().clamp(1.0, n as f64) as usize;
        v[rank - 1]
    }

    fn fmt_ms(d: Duration) -> String {
        format!("{:.3}ms", d.as_secs_f64() * 1000.0)
    }

    let fields: &[Field] = &[
        ("backend_create", |m| m.backend_create),
        ("ffmpeg_spawn", |m| m.ffmpeg_spawn),
        ("eval_total", |m| m.eval_total),
        ("compile_total", |m| m.compile_total),
        ("render_total", |m| m.render_total),
        ("encode_write_total", |m| m.encode_write_total),
        ("ffmpeg_finish", |m| m.ffmpeg_finish),
        ("wall_total", |m| m.wall_total),
    ];

    eprintln!("\npercentiles across runs (p50/p90/p99):");
    for (name, getter) in fields {
        let v = collect(runs, *getter);
        let p50 = p(&v, 0.50);
        let p90 = p(&v, 0.90);
        let p99 = p(&v, 0.99);
        eprintln!(
            "  {name:18} p50={p50:>10}  p90={p90:>10}  p99={p99:>10}",
            name = *name,
            p50 = fmt_ms(p50),
            p90 = fmt_ms(p90),
            p99 = fmt_ms(p99)
        );
    }
}
