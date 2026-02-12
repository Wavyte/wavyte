use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use serde_json::json;

#[derive(Clone, Debug)]
struct BenchArgs {
    width: u32,
    height: u32,
    fps: u32,
    seconds: u32,
    warmup: u32,
    repeats: u32,
    out_dir: PathBuf,
    keep_all_outputs: bool,
    no_encode: bool,
    parallel: bool,
    threads: Option<usize>,
    chunk_size: usize,
    static_frame_elision: bool,
    no_audio: bool,
}

#[derive(Clone, Debug, Default)]
struct RunMetrics {
    session_new: Duration,
    render_range: Duration,
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

    let out_dir = if args.out_dir.is_absolute() {
        args.out_dir.clone()
    } else {
        repo_root.join(&args.out_dir)
    };
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("create out dir '{}'", out_dir.display()))?;

    if !args.no_encode && !wavyte::encode::ffmpeg::is_ffmpeg_on_path() {
        anyhow::bail!("ffmpeg is required for encoding; install it and ensure it's on PATH");
    }

    if args.warmup > 0 {
        eprintln!("warmup: {} run(s)", args.warmup);
        for i in 0..args.warmup {
            let _ = run_once(&args, &repo_root, &out_dir, i, /*is_warmup=*/ true)?;
        }
    }

    let frames = u64::from(args.fps) * u64::from(args.seconds);
    eprintln!(
        "bench: {repeats} run(s), {frames} frames/run ({seconds}s @ {fps} fps), encode={encode}, mode={mode}, threads={threads}, chunk={chunk}, elision={elision}, audio={audio}",
        repeats = args.repeats,
        frames = frames,
        seconds = args.seconds,
        fps = args.fps,
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
        audio = if args.no_audio { "off" } else { "on" },
    );

    let mut runs = Vec::<RunMetrics>::with_capacity(args.repeats as usize);
    for i in 0..args.repeats {
        runs.push(run_once(
            &args, &repo_root, &out_dir, i, /*is_warmup=*/ false,
        )?);
    }

    report_percentiles(&runs);
    Ok(())
}

fn run_once(
    args: &BenchArgs,
    repo_root: &Path,
    out_dir: &Path,
    run_idx: u32,
    is_warmup: bool,
) -> anyhow::Result<RunMetrics> {
    let start_wall = Instant::now();
    let duration_frames = u64::from(args.fps) * u64::from(args.seconds);

    let json = build_scene_json(args, duration_frames);
    let json_bytes = serde_json::to_vec(&json).context("serialize benchmark scene json")?;

    let start_new = Instant::now();
    let comp = wavyte::Composition::from_reader(Cursor::new(json_bytes))?;
    let mut sess = wavyte::RenderSession::new(
        &comp,
        repo_root,
        wavyte::RenderSessionOpts {
            parallel: args.parallel,
            chunk_size: args.chunk_size,
            threads: args.threads,
            static_frame_elision: args.static_frame_elision,
            channel_capacity: 4,
            enable_audio: !args.no_audio,
        },
    )?;
    let t_new = start_new.elapsed();

    let out_path = out_dir.join(if is_warmup {
        "bench_warmup.mp4".to_owned()
    } else if args.keep_all_outputs {
        format!("bench_{run_idx:04}.mp4")
    } else {
        "bench_out.mp4".to_owned()
    });
    if !args.keep_all_outputs {
        let _ = std::fs::remove_file(&out_path);
    }

    let range = wavyte::FrameRange::new(wavyte::FrameIndex(0), wavyte::FrameIndex(duration_frames))
        .context("build render range")?;

    let start_render = Instant::now();
    if args.no_encode {
        let mut sink = wavyte::InMemorySink::new();
        let _stats = sess.render_range(range, wavyte::CpuBackendOpts::default(), &mut sink)?;
    } else {
        let sink_opts = wavyte::FfmpegSinkOpts {
            out_path: out_path.clone(),
            overwrite: true,
            bg_rgba: [0, 0, 0, 255],
        };
        let mut sink = wavyte::FfmpegSink::new(sink_opts);
        let _stats = sess.render_range(range, wavyte::CpuBackendOpts::default(), &mut sink)?;
    }
    let t_render = start_render.elapsed();

    Ok(RunMetrics {
        session_new: t_new,
        render_range: t_render,
        wall_total: start_wall.elapsed(),
    })
}

fn build_scene_json(args: &BenchArgs, duration_frames: u64) -> serde_json::Value {
    // Note: sources are relative to `repo_root` passed as `assets_root`.
    let title_y = (args.height as f64) / 2.0;
    json!({
      "version": "0.3",
      "canvas": { "width": args.width, "height": args.height },
      "fps": { "num": args.fps, "den": 1 },
      "duration": duration_frames,
      "assets": {
        "bg": { "solid_rect": { "color": "#18202cff" } },
        "logo": { "svg": { "source": "assets/logo.svg" } },
        "title": {
          "text": {
            "text": "wavyte v0.3 bench",
            "font_source": "wavyte/tests/data/fonts/Inconsolata-Regular.ttf",
            "size_px": 48,
            "max_width_px": (args.width as f64) - 64.0,
            "color": "#ffffff"
          }
        }
      },
      "root": {
        "id": "root",
        "range": [0, duration_frames],
        "kind": { "collection": { "mode": "stack", "children": [
          {
            "id": "bg",
            "range": [0, duration_frames],
            "kind": { "leaf": { "asset": "bg" } }
          },
          {
            "id": "logo",
            "range": [0, duration_frames],
            "kind": { "leaf": { "asset": "logo" } },
            "transform": {
              "translate": [{ "expr": "=64 + 32*sin(time.seconds*2.0)" }, 32],
              "rotation_deg": 0,
              "scale": [1, 1],
              "anchor": [0, 0],
              "skew_deg": [0, 0]
            }
          },
          {
            "id": "title",
            "range": [0, duration_frames],
            "kind": { "leaf": { "asset": "title" } },
            "transform": {
              "translate": [32, title_y],
              "rotation_deg": 0,
              "scale": [1, 1],
              "anchor": [0, 0],
              "skew_deg": [0, 0]
            },
            "opacity": { "expr": "=0.25 + 0.75*(0.5 + 0.5*sin(time.seconds*1.5))" }
          }
        ] } }
      }
    })
}

fn report_percentiles(runs: &[RunMetrics]) {
    fn p(sorted: &[Duration], q: f64) -> Duration {
        if sorted.is_empty() {
            return Duration::ZERO;
        }
        let idx = ((sorted.len() - 1) as f64 * q).round() as usize;
        sorted[idx]
    }

    let mut wall: Vec<Duration> = runs.iter().map(|r| r.wall_total).collect();
    let mut new: Vec<Duration> = runs.iter().map(|r| r.session_new).collect();
    let mut render: Vec<Duration> = runs.iter().map(|r| r.render_range).collect();
    wall.sort();
    new.sort();
    render.sort();

    eprintln!("percentiles:");
    eprintln!(
        "  session_new: p50={:?} p90={:?} p99={:?}",
        p(&new, 0.50),
        p(&new, 0.90),
        p(&new, 0.99)
    );
    eprintln!(
        "  render_range: p50={:?} p90={:?} p99={:?}",
        p(&render, 0.50),
        p(&render, 0.90),
        p(&render, 0.99)
    );
    eprintln!(
        "  wall_total: p50={:?} p90={:?} p99={:?}",
        p(&wall, 0.50),
        p(&wall, 0.90),
        p(&wall, 0.99)
    );
}

fn parse_args() -> anyhow::Result<BenchArgs> {
    let mut args = std::env::args().skip(1);

    let mut out = BenchArgs {
        width: 640,
        height: 360,
        fps: 30,
        seconds: 10,
        warmup: 1,
        repeats: 50,
        out_dir: PathBuf::from("assets/bench"),
        keep_all_outputs: false,
        no_encode: false,
        parallel: false,
        threads: None,
        chunk_size: 64,
        static_frame_elision: false,
        no_audio: false,
    };

    while let Some(a) = args.next() {
        match a.as_str() {
            "--width" => out.width = parse_u32(args.next(), "--width")?,
            "--height" => out.height = parse_u32(args.next(), "--height")?,
            "--fps" => out.fps = parse_u32(args.next(), "--fps")?,
            "--seconds" => out.seconds = parse_u32(args.next(), "--seconds")?,
            "--warmup" => out.warmup = parse_u32(args.next(), "--warmup")?,
            "--repeats" => out.repeats = parse_u32(args.next(), "--repeats")?,
            "--out-dir" => out.out_dir = PathBuf::from(require(args.next(), "--out-dir")?),
            "--keep-all-outputs" => out.keep_all_outputs = true,
            "--no-encode" => out.no_encode = true,
            "--parallel" => out.parallel = true,
            "--threads" => out.threads = Some(parse_usize(args.next(), "--threads")?),
            "--chunk-size" => out.chunk_size = parse_usize(args.next(), "--chunk-size")?,
            "--static-frame-elision" => out.static_frame_elision = true,
            "--no-audio" => out.no_audio = true,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown arg '{other}' (try --help)"),
        }
    }

    Ok(out)
}

fn print_help() {
    eprintln!(
        "\
wavyte-bench (v0.3)

Usage:
  cargo run -p wavyte-bench --release -- [args]

Args:
  --width <px>               (default 640)
  --height <px>              (default 360)
  --fps <n>                  (default 30)
  --seconds <n>              (default 10)
  --warmup <n>               (default 1)
  --repeats <n>              (default 50)
  --out-dir <path>           (default assets/bench)
  --keep-all-outputs         keep each run's output mp4
  --no-encode                render into memory (no ffmpeg)
  --parallel                 enable frame-level parallelism
  --threads <n>              override rayon worker threads
  --chunk-size <n>           frames per chunk (default 64)
  --static-frame-elision     enable elision within chunks
  --no-audio                 disable audio mixing
  --help, -h                 show this help
"
    );
}

fn require(v: Option<String>, flag: &str) -> anyhow::Result<String> {
    v.ok_or_else(|| anyhow::anyhow!("{flag} requires a value"))
}

fn parse_u32(v: Option<String>, flag: &str) -> anyhow::Result<u32> {
    let s = require(v, flag)?;
    s.parse::<u32>()
        .with_context(|| format!("parse {flag}='{s}' as u32"))
}

fn parse_usize(v: Option<String>, flag: &str) -> anyhow::Result<usize> {
    let s = require(v, flag)?;
    s.parse::<usize>()
        .with_context(|| format!("parse {flag}='{s}' as usize"))
}
