use std::path::PathBuf;

use anyhow::Context as _;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "wavyte", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Render a single frame as a PNG.
    Frame(FrameArgs),
    /// Render an MP4 video (requires `ffmpeg` on PATH).
    Render(RenderArgs),
}

#[derive(Parser, Debug)]
struct FrameArgs {
    /// Input composition JSON.
    #[arg(long = "in")]
    in_path: PathBuf,

    /// Frame index (0-based).
    #[arg(long)]
    frame: u64,

    /// Output PNG path.
    #[arg(long)]
    out: PathBuf,
}

#[derive(Parser, Debug)]
struct RenderArgs {
    /// Input composition JSON.
    #[arg(long = "in")]
    in_path: PathBuf,

    /// Output MP4 path.
    #[arg(long)]
    out: PathBuf,
    /// Overwrite output if it already exists.
    #[arg(long, default_value_t = true)]
    overwrite: bool,

    /// Enable frame-level parallelism.
    #[arg(long, default_value_t = false)]
    parallel: bool,

    /// Override rayon worker threads (parallel mode only).
    #[arg(long)]
    threads: Option<usize>,

    /// Render chunk size (parallel mode only).
    #[arg(long, default_value_t = 64)]
    chunk_size: usize,

    /// Enable static-frame elision within chunks.
    #[arg(long, default_value_t = false)]
    static_frame_elision: bool,

    /// Disable audio mixing for this render.
    #[arg(long, default_value_t = false)]
    no_audio: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Command::Frame(args) => cmd_frame(args),
        Command::Render(args) => cmd_render(args),
    }
}

fn cmd_frame(args: FrameArgs) -> anyhow::Result<()> {
    let comp = wavyte::v03::Composition::from_path(&args.in_path)?;
    let assets_root = args
        .in_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));

    let mut sess = wavyte::v03::RenderSession::new(
        &comp,
        assets_root,
        wavyte::v03::RenderSessionOpts::default(),
    )?;
    let frame = sess.render_frame(
        wavyte::FrameIndex(args.frame),
        wavyte::v03::CpuBackendOpts::default(),
    )?;

    if let Some(parent) = args.out.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create output dir '{}'", parent.display()))?;
    }

    image::save_buffer_with_format(
        &args.out,
        &frame.data,
        frame.width,
        frame.height,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .with_context(|| format!("write png '{}'", args.out.display()))?;

    eprintln!("wrote {}", args.out.display());
    Ok(())
}

fn cmd_render(args: RenderArgs) -> anyhow::Result<()> {
    let comp = wavyte::v03::Composition::from_path(&args.in_path)?;
    let assets_root = args
        .in_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));

    let opts = wavyte::v03::RenderSessionOpts {
        parallel: args.parallel,
        chunk_size: args.chunk_size,
        threads: args.threads,
        static_frame_elision: args.static_frame_elision,
        channel_capacity: 4,
        enable_audio: !args.no_audio,
    };
    let mut sess = wavyte::v03::RenderSession::new(&comp, assets_root, opts)?;

    let sink_opts = wavyte::v03::FfmpegSinkOpts {
        out_path: args.out.clone(),
        overwrite: args.overwrite,
        bg_rgba: [0, 0, 0, 255],
    };
    let mut sink = wavyte::v03::FfmpegSink::new(sink_opts);

    let range = wavyte::FrameRange::new(
        wavyte::FrameIndex(0),
        wavyte::FrameIndex(comp.duration_frames()),
    )?;
    let _stats = sess.render_range(range, wavyte::v03::CpuBackendOpts::default(), &mut sink)?;

    eprintln!("wrote {}", args.out.display());
    Ok(())
}
